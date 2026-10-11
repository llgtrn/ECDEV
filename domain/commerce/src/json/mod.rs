//! JSON for ECDEV: a strict parser, a deterministic writer, a document tree whose numbers keep
//! their text, typed conversion (`ToJson`, `FromJson`) and the `json!` macro. Objects keep their
//! keys sorted; nesting is limited to 127 containers; text that is not valid UTF-8 or not strictly
//! RFC 8259 JSON is an error, never repaired.

mod error;
mod macros;
mod number;
mod parse;
mod traits;
mod value;
mod write;

pub use error::{Category, Error, Result};
pub use number::Number;
pub use parse::{MAX_DEPTH, parse as parse_slice};
pub use traits::{
    FromJson, JsonKey, ToJson, derive_support, from_slice, from_str, from_value, invalid_type,
    to_string, to_string_pretty, to_value, to_vec,
};
pub use value::{Map, Value, ValueIndex};

/// `Map` types by their path, for code that names an entry type.
pub mod map {
    pub use super::value::Map;
    pub use std::collections::btree_map::{Entry, OccupiedEntry, VacantEntry};
}

// The derive macros share their names with the traits (different namespaces).
pub use ecdev_json_derive::{FromJson, ToJson};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn values_index_compare_and_convert() {
        let mut v = json!({"a": [1, {"b": "x"}], "n": null, "f": 1.5, "t": true});
        assert_eq!(v["a"][1]["b"], "x");
        assert_eq!(v["a"][0], 1);
        assert_eq!(
            v["missing"]["deeper"],
            Value::Null,
            "a missing member reads as null"
        );
        assert_eq!(v["a"][9], Value::Null);
        assert!(v["n"].is_null() && v["t"].as_bool() == Some(true) && v["f"].as_f64() == Some(1.5));
        v["new"]["nested"] = json!(7);
        assert_eq!(v.pointer("/new/nested"), Some(&json!(7)));
        assert_eq!(v.pointer("/a/1/b").and_then(Value::as_str), Some("x"));
        assert_eq!(v.pointer("/a/01"), None, "positions have no leading zero");
        assert_eq!(v.pointer("/a/x"), None);
        *v.pointer_mut("/a/0").unwrap() = json!("changed");
        assert_eq!(v["a"][0], "changed");
        let taken = v["a"].take();
        assert!(taken.is_array() && v["a"].is_null());
        assert_eq!(Value::from(f64::NAN), Value::Null);
        assert_eq!(Value::from(Some("s")), "s");
        assert_eq!(Value::from(None::<u8>), Value::Null);
        assert_eq!(Value::from(vec![1u8, 2]), json!([1, 2]));
        assert_eq!(
            ["a", "b"].iter().copied().collect::<Value>(),
            json!(["a", "b"])
        );
        assert_eq!([("k", 1u8)].into_iter().collect::<Value>(), json!({"k": 1}));
        assert_eq!(json!("s"), "s");
        assert_eq!("s", json!("s"));
        assert_ne!(json!(1), json!(1.0), "numbers compare by text");
    }

    #[test]
    #[should_panic(expected = "cannot access key")]
    fn indexing_a_scalar_by_key_for_writing_panics() {
        let mut v = json!(1);
        v["a"] = json!(2);
    }

    #[test]
    fn text_round_trips_and_pretty_prints() {
        let text = r#" {"b":[1,2.50,"x\u00e9\ud83d\ude00",null],"a":{"z":true,"y":-0.0}} "#;
        let v: Value = from_str(text).unwrap();
        assert_eq!(
            v.to_string(),
            "{\"a\":{\"y\":-0.0,\"z\":true},\"b\":[1,2.50,\"xé😀\",null]}"
        );
        assert_eq!(
            format!("{v:#}"),
            "{\n  \"a\": {\n    \"y\": -0.0,\n    \"z\": true\n  },\n  \"b\": [\n    1,\n    2.50,\n    \"xé😀\",\n    null\n  ]\n}"
        );
        assert_eq!(
            to_string(&"a\"\\\n\u{1}\u{7f}é").unwrap(),
            "\"a\\\"\\\\\\n\\u0001\u{7f}é\""
        );
        assert_eq!(to_string_pretty(&json!({})).unwrap(), "{}");
        assert_eq!(to_string_pretty(&json!([])).unwrap(), "[]");
    }

    #[test]
    fn errors_say_what_and_where() {
        let e = from_str::<Value>("{\"a\": 1,\n  \"b\": }").unwrap_err();
        assert!(e.is_syntax() && e.line() == 2, "{e}");
        assert!(from_str::<Value>("[1,2").unwrap_err().is_eof());
        assert!(from_str::<Value>("").unwrap_err().is_eof());
        assert!(from_str::<Value>("[1,]").is_err());
        assert!(from_str::<Value>("{'a':1}").is_err());
        assert!(from_str::<Value>("01").is_err());
        assert!(from_str::<Value>("\"\\ud800\"").is_err(), "lone surrogate");
        assert!(from_slice::<Value>(b"\"\xff\"").is_err(), "invalid UTF-8");
        assert!(from_slice::<Value>(b"[1] x").is_err());
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(
            from_str::<Value>(&deep)
                .unwrap_err()
                .to_string()
                .contains("recursion limit")
        );
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(from_str::<Value>(&ok).is_ok());
    }

    #[derive(Debug, PartialEq, ToJson, FromJson)]
    #[json(deny_unknown_fields)]
    struct Probe {
        id: u32,
        label: Option<String>,
        #[json(default)]
        tags: Vec<String>,
    }

    #[test]
    fn typed_decoding_reports_the_field() {
        let p: Probe = from_str(r#"{"id": 7}"#).unwrap();
        assert_eq!(
            p,
            Probe {
                id: 7,
                label: None,
                tags: vec![]
            }
        );
        assert_eq!(to_string(&p).unwrap(), r#"{"id":7,"label":null,"tags":[]}"#);
        let e = from_str::<Probe>(r#"{"label": "x"}"#).unwrap_err();
        assert!(e.to_string().starts_with("missing field `id`"), "{e}");
        let e = from_str::<Probe>(r#"{"id": 1, "idd": 2}"#).unwrap_err();
        assert!(
            e.to_string()
                .starts_with("unknown field `idd`, expected one of `id`, `label`, `tags`"),
            "{e}"
        );
        let e = from_str::<Probe>(r#"{"id": "1"}"#).unwrap_err();
        assert!(
            e.to_string()
                .starts_with("invalid type: string \"1\", expected u32"),
            "{e}"
        );
        let e = from_str::<Probe>(r#"{"id": -1}"#).unwrap_err();
        assert!(
            e.to_string()
                .starts_with("invalid value: integer `-1`, expected u32"),
            "{e}"
        );
        let e = from_str::<Probe>(r#"{"id": 1.5}"#).unwrap_err();
        assert!(
            e.to_string()
                .starts_with("invalid type: floating point `1.5`, expected u32"),
            "{e}"
        );
        assert!(
            from_str::<Probe>("[1,null,[]]").is_err(),
            "a struct is an object, never a sequence"
        );
    }
}
