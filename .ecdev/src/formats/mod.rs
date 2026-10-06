//! Readers for foreign formats (TOML, JSON), the deterministic JSON view writer, and the
//! Rust-expression reader that loads authoritative `.ecdev/declared/*.rs` at run time.

pub mod json;
pub mod rust;
pub mod toml;

use std::collections::BTreeMap;

/// A foreign document (TOML or JSON) after parsing.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    /// Floats are kept as written: ECDEV governance never computes with foreign floats.
    Float(String),
    Str(String),
    Array(Vec<Value>),
    Table(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Table(t) => t.get(key),
            _ => None,
        }
    }
    /// Walks a dotted path of table keys.
    pub fn at(&self, path: &str) -> Option<&Value> {
        path.split('.').try_fold(self, |v, k| v.get(k))
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
    /// A string field, empty when absent.
    pub fn text(&self, key: &str) -> String {
        match self.get(key) {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Int(n)) => n.to_string(),
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }
    pub fn items(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }
    pub fn items_of(&self, key: &str) -> &[Value] {
        self.get(key).map(Value::items).unwrap_or(&[])
    }
    /// A list of strings under `key`; a lone string is a one-element list.
    pub fn strings(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(Value::Str(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }
    pub fn table(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }
}
