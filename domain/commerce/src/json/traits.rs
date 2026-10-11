//! Typed conversion: `ToJson` builds a value from a Rust type, `FromJson` reads one back with the
//! error messages a missing field, an unknown field or a wrong type deserves. The derive macros
//! `ToJson` and `FromJson` (crate `ecdev-json-derive`) generate both for structs and enums.

use super::{
    error::{Error, Result},
    number::Number,
    value::{Map, Value},
};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    hash::{BuildHasher, Hash},
};

pub trait ToJson {
    fn to_json(&self) -> Value;
}

pub trait FromJson: Sized {
    fn from_json(v: &Value) -> Result<Self>;

    /// What an absent object member means for this type: `None` (an error) for most types,
    /// `Some(None)` for `Option`.
    fn missing() -> Option<Self> {
        None
    }
}

// ---- entry points -------------------------------------------------------------------------------

pub fn to_value<T: ToJson>(value: T) -> Result<Value> {
    Ok(value.to_json())
}

pub fn from_value<T: FromJson>(v: Value) -> Result<T> {
    T::from_json(&v)
}

pub fn to_string<T: ToJson + ?Sized>(v: &T) -> Result<String> {
    let mut out = String::new();
    super::write::compact(&v.to_json(), &mut out);
    Ok(out)
}

pub fn to_string_pretty<T: ToJson + ?Sized>(v: &T) -> Result<String> {
    let mut out = String::new();
    super::write::pretty(&v.to_json(), &mut out);
    Ok(out)
}

pub fn to_vec<T: ToJson + ?Sized>(v: &T) -> Result<Vec<u8>> {
    to_string(v).map(String::into_bytes)
}

pub fn from_slice<T: FromJson>(bytes: &[u8]) -> Result<T> {
    let v = super::parse::parse(bytes)?;
    T::from_json(&v).map_err(|e| with_position(e, bytes))
}

pub fn from_str<T: FromJson>(text: &str) -> Result<T> {
    from_slice(text.as_bytes())
}

/// A data error found after the whole text parsed carries the end of the text as its position,
/// as a document-level error does.
fn with_position(e: Error, bytes: &[u8]) -> Error {
    let line = bytes.iter().filter(|&&b| b == b'\n').count() + 1;
    let column = bytes.iter().rev().take_while(|&&b| b != b'\n').count();
    Error::custom(format_args!("{e} at line {line} column {column}"))
}

// ---- messages the way serde states them ---------------------------------------------------------

fn unexpected(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("boolean `{b}`"),
        Value::Number(n) => {
            if n.is_f64() || (!n.is_u64() && !n.is_i64()) {
                format!(
                    "floating point `{}`",
                    n.as_f64()
                        .map_or_else(|| n.as_str().to_string(), super::number::format_f64)
                )
            } else {
                format!("integer `{n}`")
            }
        }
        Value::String(s) => format!("string {s:?}"),
        Value::Array(_) => "sequence".into(),
        Value::Object(_) => "map".into(),
    }
}

pub fn invalid_type(v: &Value, expected: &str) -> Error {
    Error::custom(format_args!(
        "invalid type: {}, expected {expected}",
        unexpected(v)
    ))
}

fn out_of_range(n: &Number, expected: &str) -> Error {
    Error::custom(format_args!(
        "invalid value: integer `{n}`, expected {expected}"
    ))
}

// ---- leaves --------------------------------------------------------------------------------------

impl ToJson for Value {
    fn to_json(&self) -> Value {
        self.clone()
    }
}
impl FromJson for Value {
    fn from_json(v: &Value) -> Result<Value> {
        Ok(v.clone())
    }
}

impl ToJson for bool {
    fn to_json(&self) -> Value {
        Value::Bool(*self)
    }
}
impl FromJson for bool {
    fn from_json(v: &Value) -> Result<bool> {
        v.as_bool().ok_or_else(|| invalid_type(v, "a boolean"))
    }
}

impl ToJson for () {
    fn to_json(&self) -> Value {
        Value::Null
    }
}
impl FromJson for () {
    fn from_json(v: &Value) -> Result<()> {
        v.is_null()
            .then_some(())
            .ok_or_else(|| invalid_type(v, "unit"))
    }
}

impl ToJson for str {
    fn to_json(&self) -> Value {
        Value::String(self.to_string())
    }
}
impl ToJson for String {
    fn to_json(&self) -> Value {
        Value::String(self.clone())
    }
}
impl FromJson for String {
    fn from_json(v: &Value) -> Result<String> {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| invalid_type(v, "a string"))
    }
}
impl ToJson for Cow<'_, str> {
    fn to_json(&self) -> Value {
        Value::String(self.to_string())
    }
}
impl ToJson for char {
    fn to_json(&self) -> Value {
        Value::String(self.to_string())
    }
}
impl FromJson for char {
    fn from_json(v: &Value) -> Result<char> {
        let s = v.as_str().ok_or_else(|| invalid_type(v, "a character"))?;
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(Error::custom(format_args!(
                "invalid value: string {s:?}, expected a character"
            ))),
        }
    }
}

impl ToJson for Number {
    fn to_json(&self) -> Value {
        Value::Number(self.clone())
    }
}
impl FromJson for Number {
    fn from_json(v: &Value) -> Result<Number> {
        v.as_number()
            .cloned()
            .ok_or_else(|| invalid_type(v, "a number"))
    }
}

macro_rules! integer {
    ($($t:ty => $wide:ty, $get:ident, $name:literal;)*) => {$(
        impl ToJson for $t {
            fn to_json(&self) -> Value {
                Value::Number(Number::from(*self))
            }
        }
        impl FromJson for $t {
            fn from_json(v: &Value) -> Result<$t> {
                let Value::Number(n) = v else {
                    return Err(invalid_type(v, $name));
                };
                match n.$get() {
                    Some(wide) => <$t>::try_from(wide).map_err(|_| out_of_range(n, $name)),
                    None if n.is_f64() || n.as_str().contains(['.', 'e', 'E']) => Err(invalid_type(v, $name)),
                    // An integer beyond the type: negative for an unsigned type or too large.
                    None => Err(out_of_range(n, $name)),
                }
            }
        }
    )*};
}
integer! {
    i8 => i64, as_i64, "i8";
    i16 => i64, as_i64, "i16";
    i32 => i64, as_i64, "i32";
    i64 => i64, as_i64, "i64";
    isize => i64, as_i64, "isize";
    u8 => u64, as_u64, "u8";
    u16 => u64, as_u64, "u16";
    u32 => u64, as_u64, "u32";
    u64 => u64, as_u64, "u64";
    usize => u64, as_u64, "usize";
    i128 => i128, as_i128, "i128";
    u128 => u128, as_u128, "u128";
}

impl ToJson for f64 {
    fn to_json(&self) -> Value {
        Value::from(*self)
    }
}
impl FromJson for f64 {
    fn from_json(v: &Value) -> Result<f64> {
        match v {
            Value::Number(n) => n.as_f64().ok_or_else(|| out_of_range(n, "f64")),
            _ => Err(invalid_type(v, "f64")),
        }
    }
}
impl ToJson for f32 {
    fn to_json(&self) -> Value {
        // The shortest text that reads back as the same f32.
        if self.is_finite() {
            Value::Number(Number::from_f64(f64::from(*self)).map_or_else(
                || Number::from(0u8),
                |_| Number::from_valid_text(&shortest_f32(*self)),
            ))
        } else {
            Value::Null
        }
    }
}
impl FromJson for f32 {
    fn from_json(v: &Value) -> Result<f32> {
        f64::from_json(v).map(|f| f as f32)
    }
}

fn shortest_f32(f: f32) -> String {
    // Reuse the f64 layout on the shortest f32 digits.
    let sci = format!("{:e}", f.abs());
    let widened: f64 = sci.parse().expect("LowerExp text parses");
    let text = super::number::format_f64(widened);
    if f < 0.0 { format!("-{text}") } else { text }
}

// ---- containers ----------------------------------------------------------------------------------

impl<T: ToJson + ?Sized> ToJson for &T {
    fn to_json(&self) -> Value {
        (**self).to_json()
    }
}
impl<T: ToJson + ?Sized> ToJson for Box<T> {
    fn to_json(&self) -> Value {
        (**self).to_json()
    }
}
impl<T: FromJson> FromJson for Box<T> {
    fn from_json(v: &Value) -> Result<Box<T>> {
        T::from_json(v).map(Box::new)
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> Value {
        self.as_ref().map_or(Value::Null, ToJson::to_json)
    }
}
impl<T: FromJson> FromJson for Option<T> {
    fn from_json(v: &Value) -> Result<Option<T>> {
        if v.is_null() {
            Ok(None)
        } else {
            T::from_json(v).map(Some)
        }
    }
    fn missing() -> Option<Option<T>> {
        Some(None)
    }
}

impl<T: ToJson> ToJson for [T] {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}
impl<T: ToJson, const N: usize> ToJson for [T; N] {
    fn to_json(&self) -> Value {
        self.as_slice().to_json()
    }
}
impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> Value {
        self.as_slice().to_json()
    }
}
impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(v: &Value) -> Result<Vec<T>> {
        match v {
            Value::Array(items) => items.iter().map(T::from_json).collect(),
            _ => Err(invalid_type(v, "a sequence")),
        }
    }
}
impl<T: FromJson, const N: usize> FromJson for [T; N] {
    fn from_json(v: &Value) -> Result<[T; N]> {
        let items = Vec::<T>::from_json(v)?;
        let len = items.len();
        items.try_into().map_err(|_| {
            Error::custom(format_args!(
                "invalid length {len}, expected an array of length {N}"
            ))
        })
    }
}
impl<T: ToJson> ToJson for BTreeSet<T> {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}
impl<T: FromJson + Ord> FromJson for BTreeSet<T> {
    fn from_json(v: &Value) -> Result<BTreeSet<T>> {
        Vec::<T>::from_json(v).map(|items| items.into_iter().collect())
    }
}
impl<T: ToJson, S> ToJson for HashSet<T, S> {
    fn to_json(&self) -> Value {
        Value::Array(self.iter().map(ToJson::to_json).collect())
    }
}
impl<T: FromJson + Eq + Hash, S: BuildHasher + Default> FromJson for HashSet<T, S> {
    fn from_json(v: &Value) -> Result<HashSet<T, S>> {
        Vec::<T>::from_json(v).map(|items| items.into_iter().collect())
    }
}

impl ToJson for Map<String, Value> {
    fn to_json(&self) -> Value {
        Value::Object(self.clone())
    }
}
impl FromJson for Map<String, Value> {
    fn from_json(v: &Value) -> Result<Map<String, Value>> {
        v.as_object()
            .cloned()
            .ok_or_else(|| invalid_type(v, "a map"))
    }
}

/// Map keys are text in JSON.
pub trait JsonKey {
    fn json_key(&self) -> String;
}
impl JsonKey for String {
    fn json_key(&self) -> String {
        self.clone()
    }
}
impl JsonKey for str {
    fn json_key(&self) -> String {
        self.to_string()
    }
}
impl JsonKey for Cow<'_, str> {
    fn json_key(&self) -> String {
        self.to_string()
    }
}
impl<K: JsonKey + ?Sized> JsonKey for &K {
    fn json_key(&self) -> String {
        (**self).json_key()
    }
}
macro_rules! integer_keys {
    ($($t:ty),*) => {$(
        impl JsonKey for $t {
            fn json_key(&self) -> String {
                self.to_string()
            }
        }
    )*};
}
integer_keys!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

impl<K: JsonKey, T: ToJson> ToJson for BTreeMap<K, T> {
    fn to_json(&self) -> Value {
        Value::Object(
            self.iter()
                .map(|(k, v)| (k.json_key(), v.to_json()))
                .collect(),
        )
    }
}
impl<T: FromJson> FromJson for BTreeMap<String, T> {
    fn from_json(v: &Value) -> Result<BTreeMap<String, T>> {
        match v {
            Value::Object(m) => m
                .iter()
                .map(|(k, v)| Ok((k.clone(), T::from_json(v)?)))
                .collect(),
            _ => Err(invalid_type(v, "a map")),
        }
    }
}
impl<K: JsonKey, T: ToJson, S> ToJson for HashMap<K, T, S> {
    fn to_json(&self) -> Value {
        Value::Object(
            self.iter()
                .map(|(k, v)| (k.json_key(), v.to_json()))
                .collect(),
        )
    }
}
impl<T: FromJson, S: BuildHasher + Default> FromJson for HashMap<String, T, S> {
    fn from_json(v: &Value) -> Result<HashMap<String, T, S>> {
        match v {
            Value::Object(m) => m
                .iter()
                .map(|(k, v)| Ok((k.clone(), T::from_json(v)?)))
                .collect(),
            _ => Err(invalid_type(v, "a map")),
        }
    }
}

macro_rules! tuple {
    ($(($($n:tt $t:ident),+) $len:literal;)*) => {$(
        impl<$($t: ToJson),+> ToJson for ($($t,)+) {
            fn to_json(&self) -> Value {
                Value::Array(vec![$(self.$n.to_json()),+])
            }
        }
        impl<$($t: FromJson),+> FromJson for ($($t,)+) {
            fn from_json(v: &Value) -> Result<Self> {
                match v {
                    Value::Array(items) if items.len() == $len => Ok(($($t::from_json(&items[$n])?,)+)),
                    Value::Array(items) => Err(Error::custom(format_args!(
                        "invalid length {}, expected a tuple of size {}", items.len(), $len))),
                    _ => Err(invalid_type(v, concat!("a tuple of size ", $len))),
                }
            }
        }
    )*};
}
tuple! {
    (0 A) 1;
    (0 A, 1 B) 2;
    (0 A, 1 B, 2 C) 3;
    (0 A, 1 B, 2 C, 3 D) 4;
}

// ---- helpers the derive macros call -------------------------------------------------------------

#[doc(hidden)]
pub mod derive_support {
    use super::*;

    pub fn object<'v>(v: &'v Value, expected: &str) -> Result<&'v Map<String, Value>> {
        v.as_object().ok_or_else(|| invalid_type(v, expected))
    }

    pub fn field<T: FromJson>(obj: &Map<String, Value>, name: &str) -> Result<T> {
        match obj.get(name) {
            Some(v) => T::from_json(v),
            None => {
                T::missing().ok_or_else(|| Error::custom(format_args!("missing field `{name}`")))
            }
        }
    }

    pub fn field_or_else<T: FromJson>(
        obj: &Map<String, Value>,
        name: &str,
        default: impl FnOnce() -> T,
    ) -> Result<T> {
        match obj.get(name) {
            Some(v) => T::from_json(v),
            None => Ok(default()),
        }
    }

    pub fn deny_unknown(obj: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
        for key in obj.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(Error::custom(format_args!(
                    "unknown field `{key}`, {}",
                    expected_list(allowed, "field")
                )));
            }
        }
        Ok(())
    }

    pub fn expected_list(names: &[&str], noun: &str) -> String {
        match names {
            [] => format!("there are no {noun}s"),
            [one] => format!("expected `{one}`"),
            [a, b] => format!("expected `{a}` or `{b}`"),
            many => {
                let joined: Vec<String> = many.iter().map(|n| format!("`{n}`")).collect();
                format!("expected one of {}", joined.join(", "))
            }
        }
    }

    pub fn unknown_variant(name: &str, variants: &[&str]) -> Error {
        Error::custom(format_args!(
            "unknown variant `{name}`, {}",
            expected_list(variants, "variant")
        ))
    }

    pub fn tag<'v>(obj: &'v Map<String, Value>, name: &str) -> Result<&'v str> {
        match obj.get(name) {
            None => Err(Error::custom(format_args!("missing field `{name}`"))),
            Some(Value::String(s)) => Ok(s),
            Some(other) => Err(invalid_type(other, "a string")),
        }
    }

    pub fn string_variant(v: &Value) -> Result<&str> {
        v.as_str().ok_or_else(|| invalid_type(v, "a string"))
    }
}
