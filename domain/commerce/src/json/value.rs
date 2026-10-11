//! The JSON document tree.

use super::number::Number;
use std::{borrow::Cow, collections::BTreeMap, fmt, ops};

/// A JSON object. Keys stay sorted, so a serialised document is deterministic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map<K = String, V = Value> {
    inner: BTreeMap<K, V>,
}

impl Default for Map<String, Value> {
    fn default() -> Self {
        Map::new()
    }
}

impl Map<String, Value> {
    pub fn new() -> Self {
        Map {
            inner: BTreeMap::new(),
        }
    }
    pub fn insert(&mut self, key: String, value: Value) -> Option<Value> {
        self.inner.insert(key, value)
    }
    pub fn get<Q: ?Sized + Ord>(&self, key: &Q) -> Option<&Value>
    where
        String: std::borrow::Borrow<Q>,
    {
        self.inner.get(key)
    }
    pub fn get_mut<Q: ?Sized + Ord>(&mut self, key: &Q) -> Option<&mut Value>
    where
        String: std::borrow::Borrow<Q>,
    {
        self.inner.get_mut(key)
    }
    pub fn contains_key<Q: ?Sized + Ord>(&self, key: &Q) -> bool
    where
        String: std::borrow::Borrow<Q>,
    {
        self.inner.contains_key(key)
    }
    pub fn remove<Q: ?Sized + Ord>(&mut self, key: &Q) -> Option<Value>
    where
        String: std::borrow::Borrow<Q>,
    {
        self.inner.remove(key)
    }
    pub fn entry<S: Into<String>>(
        &mut self,
        key: S,
    ) -> std::collections::btree_map::Entry<'_, String, Value> {
        self.inner.entry(key.into())
    }
    pub fn into_keys(self) -> std::collections::btree_map::IntoKeys<String, Value> {
        self.inner.into_keys()
    }
    pub fn into_values(self) -> std::collections::btree_map::IntoValues<String, Value> {
        self.inner.into_values()
    }
    pub fn append(&mut self, other: &mut Map<String, Value>) {
        self.inner.append(&mut other.inner)
    }
}

impl<K, V> Map<K, V> {
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    pub fn clear(&mut self) {
        self.inner.clear()
    }
    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, K, V> {
        self.inner.iter()
    }
    pub fn iter_mut(&mut self) -> std::collections::btree_map::IterMut<'_, K, V> {
        self.inner.iter_mut()
    }
    pub fn keys(&self) -> std::collections::btree_map::Keys<'_, K, V> {
        self.inner.keys()
    }
    pub fn values(&self) -> std::collections::btree_map::Values<'_, K, V> {
        self.inner.values()
    }
    pub fn values_mut(&mut self) -> std::collections::btree_map::ValuesMut<'_, K, V> {
        self.inner.values_mut()
    }
}

impl<K: Ord, V> Map<K, V> {
    pub fn retain<F: FnMut(&K, &mut V) -> bool>(&mut self, f: F) {
        self.inner.retain(f)
    }
}

impl<K, V> IntoIterator for Map<K, V> {
    type Item = (K, V);
    type IntoIter = std::collections::btree_map::IntoIter<K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}
impl<'a, K, V> IntoIterator for &'a Map<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
    }
}
impl<'a, K, V> IntoIterator for &'a mut Map<K, V> {
    type Item = (&'a K, &'a mut V);
    type IntoIter = std::collections::btree_map::IterMut<'a, K, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter_mut()
    }
}
impl FromIterator<(String, Value)> for Map<String, Value> {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Map {
            inner: iter.into_iter().collect(),
        }
    }
}
impl Extend<(String, Value)> for Map<String, Value> {
    fn extend<I: IntoIterator<Item = (String, Value)>>(&mut self, iter: I) {
        self.inner.extend(iter)
    }
}
impl From<BTreeMap<String, Value>> for Map<String, Value> {
    fn from(inner: BTreeMap<String, Value>) -> Self {
        Map { inner }
    }
}
impl From<Map<String, Value>> for BTreeMap<String, Value> {
    fn from(map: Map<String, Value>) -> Self {
        map.inner
    }
}
impl<Q: ?Sized + Ord> ops::IndexMut<&Q> for Map<String, Value>
where
    String: std::borrow::Borrow<Q>,
{
    fn index_mut(&mut self, key: &Q) -> &mut Value {
        self.inner.get_mut(key).expect("no entry found for key")
    }
}
impl<Q: ?Sized + Ord> ops::Index<&Q> for Map<String, Value>
where
    String: std::borrow::Borrow<Q>,
{
    type Output = Value;
    fn index(&self, key: &Q) -> &Value {
        self.inner.get(key).expect("no entry found for key")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Value>),
    Object(Map<String, Value>),
}

static NULL: Value = Value::Null;

/// What can select a part of a value: a key (`&str`, `String`) or a position (`usize`).
pub trait ValueIndex: private::Sealed {
    #[doc(hidden)]
    fn get<'v>(&self, v: &'v Value) -> Option<&'v Value>;
    #[doc(hidden)]
    fn get_mut<'v>(&self, v: &'v mut Value) -> Option<&'v mut Value>;
    #[doc(hidden)]
    fn index_or_insert<'v>(&self, v: &'v mut Value) -> &'v mut Value;
}

mod private {
    pub trait Sealed {}
    impl Sealed for usize {}
    impl Sealed for str {}
    impl Sealed for String {}
    impl<T: ?Sized + Sealed> Sealed for &T {}
}

impl ValueIndex for usize {
    fn get<'v>(&self, v: &'v Value) -> Option<&'v Value> {
        match v {
            Value::Array(a) => a.get(*self),
            _ => None,
        }
    }
    fn get_mut<'v>(&self, v: &'v mut Value) -> Option<&'v mut Value> {
        match v {
            Value::Array(a) => a.get_mut(*self),
            _ => None,
        }
    }
    fn index_or_insert<'v>(&self, v: &'v mut Value) -> &'v mut Value {
        match v {
            Value::Array(a) => {
                let len = a.len();
                a.get_mut(*self).unwrap_or_else(|| {
                    panic!("cannot access index {} of JSON array of length {len}", self)
                })
            }
            _ => panic!("cannot access index {} of JSON {}", self, v.kind()),
        }
    }
}

impl ValueIndex for str {
    fn get<'v>(&self, v: &'v Value) -> Option<&'v Value> {
        match v {
            Value::Object(m) => m.get(self),
            _ => None,
        }
    }
    fn get_mut<'v>(&self, v: &'v mut Value) -> Option<&'v mut Value> {
        match v {
            Value::Object(m) => m.get_mut(self),
            _ => None,
        }
    }
    fn index_or_insert<'v>(&self, v: &'v mut Value) -> &'v mut Value {
        if let Value::Null = v {
            *v = Value::Object(Map::new());
        }
        match v {
            Value::Object(m) => m.entry(self.to_string()).or_insert(Value::Null),
            _ => panic!("cannot access key {:?} in JSON {}", self, v.kind()),
        }
    }
}

impl ValueIndex for String {
    fn get<'v>(&self, v: &'v Value) -> Option<&'v Value> {
        <str as ValueIndex>::get(self.as_str(), v)
    }
    fn get_mut<'v>(&self, v: &'v mut Value) -> Option<&'v mut Value> {
        <str as ValueIndex>::get_mut(self.as_str(), v)
    }
    fn index_or_insert<'v>(&self, v: &'v mut Value) -> &'v mut Value {
        <str as ValueIndex>::index_or_insert(self.as_str(), v)
    }
}

impl<T: ?Sized + ValueIndex> ValueIndex for &T {
    fn get<'v>(&self, v: &'v Value) -> Option<&'v Value> {
        (**self).get(v)
    }
    fn get_mut<'v>(&self, v: &'v mut Value) -> Option<&'v mut Value> {
        (**self).get_mut(v)
    }
    fn index_or_insert<'v>(&self, v: &'v mut Value) -> &'v mut Value {
        (**self).index_or_insert(v)
    }
}

impl<I: ValueIndex> ops::Index<I> for Value {
    type Output = Value;
    fn index(&self, index: I) -> &Value {
        index.get(self).unwrap_or(&NULL)
    }
}

impl<I: ValueIndex> ops::IndexMut<I> for Value {
    fn index_mut(&mut self, index: I) -> &mut Value {
        index.index_or_insert(self)
    }
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    pub fn get<I: ValueIndex>(&self, index: I) -> Option<&Value> {
        index.get(self)
    }

    pub fn get_mut<I: ValueIndex>(&mut self, index: I) -> Option<&mut Value> {
        index.get_mut(self)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_boolean(&self) -> bool {
        matches!(self, Value::Bool(_))
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
    pub fn is_i64(&self) -> bool {
        self.as_number().is_some_and(Number::is_i64)
    }
    pub fn is_u64(&self) -> bool {
        self.as_number().is_some_and(Number::is_u64)
    }
    pub fn is_f64(&self) -> bool {
        self.as_number().is_some_and(Number::is_f64)
    }

    pub fn as_null(&self) -> Option<()> {
        self.is_null().then_some(())
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.as_number().and_then(Number::as_i64)
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_number().and_then(Number::as_u64)
    }
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number().and_then(Number::as_f64)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_object(&self) -> Option<&Map<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_object_mut(&mut self) -> Option<&mut Map<String, Value>> {
        match self {
            Value::Object(m) => Some(m),
            _ => None,
        }
    }

    /// Looks up a part by JSON Pointer (RFC 6901): `""` is the value itself, `/a/0` its key `a`, then
    /// position 0.
    pub fn pointer(&self, pointer: &str) -> Option<&Value> {
        if pointer.is_empty() {
            return Some(self);
        }
        if !pointer.starts_with('/') {
            return None;
        }
        pointer[1..]
            .split('/')
            .map(|part| part.replace("~1", "/").replace("~0", "~"))
            .try_fold(self, |target, token| match target {
                Value::Object(m) => m.get(&token),
                Value::Array(a) => parse_index(&token).and_then(|i| a.get(i)),
                _ => None,
            })
    }

    pub fn pointer_mut(&mut self, pointer: &str) -> Option<&mut Value> {
        if pointer.is_empty() {
            return Some(self);
        }
        if !pointer.starts_with('/') {
            return None;
        }
        pointer[1..]
            .split('/')
            .map(|part| part.replace("~1", "/").replace("~0", "~"))
            .try_fold(self, |target, token| match target {
                Value::Object(m) => m.get_mut(&token),
                Value::Array(a) => parse_index(&token).and_then(|i| a.get_mut(i)),
                _ => None,
            })
    }

    /// Replaces the value with null and returns what it held.
    pub fn take(&mut self) -> Value {
        std::mem::take(self)
    }
}

fn parse_index(s: &str) -> Option<usize> {
    if s.starts_with('+') || (s.starts_with('0') && s.len() != 1) {
        return None;
    }
    s.parse().ok()
}

impl fmt::Display for Value {
    /// Compact text; the alternate flag (`{:#}`) gives indented text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        if f.alternate() {
            super::write::pretty(self, &mut out);
        } else {
            super::write::compact(self, &mut out);
        }
        f.write_str(&out)
    }
}

// ---- construction ------------------------------------------------------------------------------

macro_rules! from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(n: $t) -> Value {
                Value::Number(Number::from(n))
            }
        }
    )*};
}
from_number!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

impl From<f64> for Value {
    /// A float that is not finite becomes null, as JSON has no text for it.
    fn from(f: f64) -> Value {
        Number::from_f64(f).map_or(Value::Null, Value::Number)
    }
}
impl From<f32> for Value {
    fn from(f: f32) -> Value {
        Value::from(f64::from(f))
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<Number> for Value {
    fn from(n: Number) -> Value {
        Value::Number(n)
    }
}
impl From<String> for Value {
    fn from(s: String) -> Value {
        Value::String(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::String(s.to_string())
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Value {
        Value::String(s.clone())
    }
}
impl From<Cow<'_, str>> for Value {
    fn from(s: Cow<'_, str>) -> Value {
        Value::String(s.into_owned())
    }
}
impl From<Map<String, Value>> for Value {
    fn from(m: Map<String, Value>) -> Value {
        Value::Object(m)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Value {
        Value::Array(v.into_iter().map(Into::into).collect())
    }
}
impl<T: Clone + Into<Value>> From<&[T]> for Value {
    fn from(v: &[T]) -> Value {
        Value::Array(v.iter().cloned().map(Into::into).collect())
    }
}
impl From<()> for Value {
    fn from((): ()) -> Value {
        Value::Null
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Value {
        o.map_or(Value::Null, Into::into)
    }
}
impl<T: Into<Value>> FromIterator<T> for Value {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Value {
        Value::Array(iter.into_iter().map(Into::into).collect())
    }
}
impl<K: Into<String>, V: Into<Value>> FromIterator<(K, V)> for Value {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Value {
        Value::Object(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

// ---- comparison with plain values --------------------------------------------------------------

impl PartialEq<str> for Value {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == Some(other)
    }
}
impl PartialEq<&str> for Value {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == Some(*other)
    }
}
impl PartialEq<String> for Value {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == Some(other.as_str())
    }
}
impl PartialEq<Value> for str {
    fn eq(&self, other: &Value) -> bool {
        other.as_str() == Some(self)
    }
}
impl PartialEq<Value> for &str {
    fn eq(&self, other: &Value) -> bool {
        other.as_str() == Some(*self)
    }
}
impl PartialEq<Value> for String {
    fn eq(&self, other: &Value) -> bool {
        other.as_str() == Some(self.as_str())
    }
}
impl PartialEq<bool> for Value {
    fn eq(&self, other: &bool) -> bool {
        self.as_bool() == Some(*other)
    }
}
impl PartialEq<Value> for bool {
    fn eq(&self, other: &Value) -> bool {
        other.as_bool() == Some(*self)
    }
}

macro_rules! eq_signed {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Value {
            fn eq(&self, other: &$t) -> bool {
                self.as_i64() == Some(i64::from(*other))
            }
        }
        impl PartialEq<Value> for $t {
            fn eq(&self, other: &Value) -> bool {
                other.as_i64() == Some(i64::from(*self))
            }
        }
    )*};
}
macro_rules! eq_unsigned {
    ($($t:ty),*) => {$(
        impl PartialEq<$t> for Value {
            fn eq(&self, other: &$t) -> bool {
                self.as_u64() == Some(u64::from(*other))
            }
        }
        impl PartialEq<Value> for $t {
            fn eq(&self, other: &Value) -> bool {
                other.as_u64() == Some(u64::from(*self))
            }
        }
    )*};
}
eq_signed!(i8, i16, i32, i64);
eq_unsigned!(u8, u16, u32, u64);

impl PartialEq<isize> for Value {
    fn eq(&self, other: &isize) -> bool {
        self.as_i64() == i64::try_from(*other).ok()
    }
}
impl PartialEq<usize> for Value {
    fn eq(&self, other: &usize) -> bool {
        self.as_u64() == u64::try_from(*other).ok()
    }
}
impl PartialEq<f64> for Value {
    fn eq(&self, other: &f64) -> bool {
        self.as_f64() == Some(*other)
    }
}
impl PartialEq<Value> for f64 {
    fn eq(&self, other: &Value) -> bool {
        other.as_f64() == Some(*self)
    }
}
impl PartialEq<f32> for Value {
    fn eq(&self, other: &f32) -> bool {
        self.as_f64() == Some(f64::from(*other))
    }
}
