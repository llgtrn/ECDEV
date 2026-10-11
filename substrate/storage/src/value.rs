//! SQL values with SQLite's dynamic typing: five storage classes, column affinity, and the
//! ordering and comparison rules the queries depend on.

use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Integer,
    Real,
    Numeric,
    Text,
    Blob,
}

impl Affinity {
    /// SQLite's rules for the affinity of a declared column type.
    pub fn of_declared(decl: &str) -> Affinity {
        let d = decl.to_ascii_uppercase();
        if d.contains("INT") {
            Affinity::Integer
        } else if d.contains("CHAR") || d.contains("CLOB") || d.contains("TEXT") {
            Affinity::Text
        } else if d.is_empty() || d.contains("BLOB") {
            Affinity::Blob
        } else if d.contains("REAL") || d.contains("FLOA") || d.contains("DOUB") {
            Affinity::Real
        } else {
            Affinity::Numeric
        }
    }

    fn is_numeric(self) -> bool {
        matches!(self, Affinity::Integer | Affinity::Real | Affinity::Numeric)
    }
}

impl Value {
    /// A real number; NaN is stored as NULL, as SQLite does.
    pub fn real(f: f64) -> Value {
        if f.is_nan() {
            Value::Null
        } else {
            Value::Real(f)
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    fn class_rank(&self) -> u8 {
        match self {
            Value::Null => 0,
            Value::Integer(_) | Value::Real(_) => 1,
            Value::Text(_) => 2,
            Value::Blob(_) => 3,
        }
    }

    /// The total order of SQLite: NULL, then numbers (by value), then text (bytewise), then blobs.
    pub fn compare(&self, other: &Value) -> Ordering {
        match (self, other) {
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Integer(a), Value::Real(b)) => cmp_int_real(*a, *b),
            (Value::Real(a), Value::Integer(b)) => cmp_int_real(*b, *a).reverse(),
            (Value::Real(a), Value::Real(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Text(a), Value::Text(b)) => a.as_bytes().cmp(b.as_bytes()),
            (Value::Blob(a), Value::Blob(b)) => a.cmp(b),
            (a, b) => a.class_rank().cmp(&b.class_rank()),
        }
    }

    /// Converts a stored value for a column of this affinity, as SQLite does on insert.
    pub fn apply_affinity(self, affinity: Affinity) -> Value {
        match (affinity, self) {
            (Affinity::Blob, v) => v,
            (_, Value::Null) => Value::Null,
            (Affinity::Text, Value::Integer(i)) => Value::Text(i.to_string()),
            (Affinity::Text, Value::Real(f)) => Value::Text(format_real(f)),
            (Affinity::Text, v) => v,
            (a, Value::Text(s)) if a.is_numeric() => match parse_numeric(&s) {
                Some(Numeric::Int(i)) => {
                    if a == Affinity::Real {
                        Value::Real(i as f64)
                    } else {
                        Value::Integer(i)
                    }
                }
                Some(Numeric::Real(f)) => real_or_integer(f, a),
                None => Value::Text(s),
            },
            (Affinity::Real, Value::Integer(i)) => Value::Real(i as f64),
            (Affinity::Integer | Affinity::Numeric, Value::Real(f)) => real_or_integer(f, affinity),
            (_, v) => v,
        }
    }

    /// Truthiness in WHERE and boolean contexts: NULL is unknown, numbers are true when nonzero.
    pub fn truth(&self) -> Option<bool> {
        match self {
            Value::Null => None,
            Value::Integer(i) => Some(*i != 0),
            Value::Real(f) => Some(*f != 0.0),
            Value::Text(s) => Some(match parse_numeric_prefix(s) {
                Numeric::Int(i) => i != 0,
                Numeric::Real(f) => f != 0.0,
            }),
            Value::Blob(b) => Some(match parse_numeric_prefix(&String::from_utf8_lossy(b)) {
                Numeric::Int(i) => i != 0,
                Numeric::Real(f) => f != 0.0,
            }),
        }
    }

    pub fn from_bool(b: bool) -> Value {
        Value::Integer(i64::from(b))
    }

    /// Numeric value for arithmetic: text and blobs use their numeric prefix.
    pub fn to_numeric(&self) -> Option<Numeric> {
        match self {
            Value::Null => None,
            Value::Integer(i) => Some(Numeric::Int(*i)),
            Value::Real(f) => Some(Numeric::Real(*f)),
            Value::Text(s) => Some(parse_numeric_prefix(s)),
            Value::Blob(b) => Some(parse_numeric_prefix(&String::from_utf8_lossy(b))),
        }
    }

    pub fn to_text(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Integer(i) => Some(i.to_string()),
            Value::Real(f) => Some(format_real(*f)),
            Value::Text(s) => Some(s.clone()),
            Value::Blob(b) => Some(String::from_utf8_lossy(b).into_owned()),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Integer(_) => "integer",
            Value::Real(_) => "real",
            Value::Text(_) => "text",
            Value::Blob(_) => "blob",
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        self.compare(other) == Ordering::Equal
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Numeric {
    Int(i64),
    Real(f64),
}

impl Numeric {
    pub fn to_value(self) -> Value {
        match self {
            Numeric::Int(i) => Value::Integer(i),
            Numeric::Real(f) => Value::Real(f),
        }
    }
    pub fn as_f64(self) -> f64 {
        match self {
            Numeric::Int(i) => i as f64,
            Numeric::Real(f) => f,
        }
    }
}

fn cmp_int_real(i: i64, f: f64) -> Ordering {
    if f.is_nan() {
        return Ordering::Greater;
    }
    if f >= 9.223_372_036_854_776e18 {
        return Ordering::Less;
    }
    if f < -9.223_372_036_854_776e18 {
        return Ordering::Greater;
    }
    let t = f.trunc();
    match i.cmp(&(t as i64)) {
        Ordering::Equal => 0.0f64.partial_cmp(&(f - t)).unwrap_or(Ordering::Equal),
        other => other,
    }
}

fn real_or_integer(f: f64, affinity: Affinity) -> Value {
    if affinity != Affinity::Real && f.fract() == 0.0 && f.abs() < 9.223_372_036_854_776e18 {
        Value::Integer(f as i64)
    } else {
        Value::Real(f)
    }
}

/// A whole string that is an integer or real number (leading and trailing blanks allowed).
pub fn parse_numeric(s: &str) -> Option<Numeric> {
    let t =
        s.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\u{c}');
    if t.is_empty() {
        return None;
    }
    let (n, used) = numeric_prefix(t)?;
    (used == t.len()).then_some(n)
}

/// The longest numeric prefix of a string, or zero.
pub fn parse_numeric_prefix(s: &str) -> Numeric {
    let t = s.trim_start_matches(|c: char| {
        c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\u{c}'
    });
    numeric_prefix(t).map_or(Numeric::Int(0), |(n, _)| n)
}

fn numeric_prefix(t: &str) -> Option<(Numeric, usize)> {
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - digits_start;
    let mut real = false;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        let mut j = i + 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        frac_digits = j - i - 1;
        if int_digits > 0 || frac_digits > 0 {
            real = true;
            i = j;
        }
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            real = true;
            i = j;
        }
    }
    let text = &t[..i];
    if !real && let Ok(v) = text.parse::<i64>() {
        return Some((Numeric::Int(v), i));
    }
    text.parse::<f64>().ok().map(|f| (Numeric::Real(f), i))
}

/// SQLite prints a real with at most 15 significant digits and always a decimal point.
pub fn format_real(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    let s = format!("{:.15e}", f);
    // %!.15g: 15 significant digits, then trim.
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let neg = mant.starts_with('-');
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-4..15).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
            out.push_str(digits);
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    }
    out
}

/// Whether a comparison applies numeric or text affinity to an operand, SQLite's rule.
pub fn coerce_for_comparison(
    a: Value,
    a_aff: Option<Affinity>,
    b: Value,
    b_aff: Option<Affinity>,
) -> (Value, Value) {
    let numeric = |x: Option<Affinity>| x.is_some_and(Affinity::is_numeric);
    let textual = |x: Option<Affinity>| x == Some(Affinity::Text);
    let none = |x: Option<Affinity>| x.is_none() || x == Some(Affinity::Blob);
    if numeric(a_aff) && (textual(b_aff) || none(b_aff)) {
        return (a, b.apply_affinity(Affinity::Numeric));
    }
    if numeric(b_aff) && (textual(a_aff) || none(a_aff)) {
        return (a.apply_affinity(Affinity::Numeric), b);
    }
    if textual(a_aff) && none(b_aff) {
        return (a, b.apply_affinity(Affinity::Text));
    }
    if textual(b_aff) && none(a_aff) {
        return (a.apply_affinity(Affinity::Text), b);
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_follows_storage_classes() {
        let mut v = [
            Value::Blob(vec![1]),
            Value::Text("b".into()),
            Value::Real(2.5),
            Value::Integer(3),
            Value::Null,
            Value::Integer(-1),
            Value::Text("a".into()),
        ];
        v.sort_by(Value::compare);
        let names: Vec<_> = v.iter().map(Value::type_name).collect();
        assert_eq!(
            names,
            ["null", "integer", "real", "integer", "text", "text", "blob"]
        );
        assert_eq!(
            Value::Integer(2).compare(&Value::Real(2.0)),
            Ordering::Equal
        );
        assert_eq!(
            Value::Integer(i64::MAX).compare(&Value::Real(9.3e18)),
            Ordering::Less
        );
    }

    #[test]
    fn affinity_converts_on_storage() {
        assert_eq!(
            Value::Text("12".into()).apply_affinity(Affinity::Integer),
            Value::Integer(12)
        );
        assert_eq!(
            Value::Text("12.5".into()).apply_affinity(Affinity::Integer),
            Value::Real(12.5)
        );
        assert_eq!(
            Value::Text("12.0".into()).apply_affinity(Affinity::Integer),
            Value::Integer(12)
        );
        assert_eq!(
            Value::Text("abc".into()).apply_affinity(Affinity::Integer),
            Value::Text("abc".into())
        );
        assert_eq!(
            Value::Integer(7).apply_affinity(Affinity::Text),
            Value::Text("7".into())
        );
        assert_eq!(
            Value::Integer(7).apply_affinity(Affinity::Real),
            Value::Real(7.0)
        );
        assert_eq!(Affinity::of_declared("INTEGER"), Affinity::Integer);
        assert_eq!(Affinity::of_declared("VARCHAR(10)"), Affinity::Text);
        assert_eq!(Affinity::of_declared(""), Affinity::Blob);
        assert_eq!(Affinity::of_declared("DOUBLE"), Affinity::Real);
        assert_eq!(Affinity::of_declared("DECIMAL"), Affinity::Numeric);
    }

    #[test]
    fn reals_print_as_sqlite_prints_them() {
        for (f, s) in [
            (1.0, "1.0"),
            (0.5, "0.5"),
            (100.0, "100.0"),
            (1e15, "1.0e+15"),
            (123456789012345.0, "123456789012345.0"),
            (0.0001, "0.0001"),
            (0.00001, "1.0e-05"),
            (-2.5, "-2.5"),
            (3.25159, "3.25159"),
        ] {
            assert_eq!(format_real(f), s, "{f}");
        }
    }
}
