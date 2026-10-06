//! schema.org ItemAvailability as a DERIVED class beside the observed value. The observed string
//! is never rewritten; spellings of one term ("InStock", "https://schema.org/InStock",
//! "schema:InStock", "http://schema.org/InStock/") are one term, and an unrecognised value is
//! UNRECOGNISED, never guessed into stock or out of it.

use serde_json::{Value, json};

/// (term, class). Classes say what a buyer can do now, not how many units exist.
const TERMS: &[(&str, &str)] = &[
    ("InStock", "AVAILABLE_ONLINE"),
    ("OnlineOnly", "AVAILABLE_ONLINE"),
    ("LimitedAvailability", "AVAILABLE_ONLINE"),
    ("InStoreOnly", "IN_STORE_ONLY"),
    ("PreOrder", "ORDERABLE_LATER"),
    ("PreSale", "ORDERABLE_LATER"),
    ("BackOrder", "ORDERABLE_LATER"),
    ("MadeToOrder", "ORDERABLE_LATER"),
    ("OutOfStock", "NOT_AVAILABLE"),
    ("SoldOut", "NOT_AVAILABLE"),
    ("Discontinued", "NOT_AVAILABLE"),
    ("Reserved", "NOT_AVAILABLE"),
];

/// The schema.org term an observed availability value names, if it names one.
pub fn availability_term(observed: &Value) -> Option<&'static str> {
    let s = observed.as_str()?.trim().trim_end_matches('/');
    let local = ["https://schema.org/", "http://schema.org/", "schema:"]
        .iter()
        .find_map(|p| {
            s.get(..p.len())
                .filter(|h| h.eq_ignore_ascii_case(p))
                .map(|_| &s[p.len()..])
        })
        .unwrap_or(s);
    TERMS
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(local))
        .map(|(t, _)| *t)
}

pub fn availability_class(observed: &Value) -> &'static str {
    availability_term(observed)
        .and_then(|t| TERMS.iter().find(|(x, _)| *x == t))
        .map_or("UNRECOGNISED", |(_, c)| c)
}

/// The observed value with its derived term and class.
pub fn derived_availability(observed: &Value) -> Value {
    json!({"observed":observed,"term":availability_term(observed),"class":availability_class(observed),"state":"DERIVED","basis":"SCHEMA_ORG_ITEM_AVAILABILITY_TERM"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_term_are_one_term() {
        for s in [
            "InStock",
            "https://schema.org/InStock",
            "http://schema.org/InStock/",
            "schema:InStock",
            " instock ",
            "HTTPS://SCHEMA.ORG/InStock",
        ] {
            assert_eq!(availability_term(&json!(s)), Some("InStock"), "{s}");
            assert_eq!(availability_class(&json!(s)), "AVAILABLE_ONLINE");
        }
        assert_eq!(
            availability_class(&json!("https://schema.org/PreOrder")),
            "ORDERABLE_LATER"
        );
        assert_eq!(availability_class(&json!("SoldOut")), "NOT_AVAILABLE");
        assert_eq!(availability_class(&json!("InStoreOnly")), "IN_STORE_ONLY");
    }

    #[test]
    fn unrecognised_values_are_never_guessed() {
        for v in [
            json!("In stock"),
            json!("https://example.com/InStock"),
            json!("available"),
            json!(true),
            Value::Null,
            json!(""),
        ] {
            assert_eq!(availability_term(&v), None, "{v}");
            assert_eq!(availability_class(&v), "UNRECOGNISED");
        }
        let d = derived_availability(&json!("schema:OutOfStock"));
        assert_eq!(
            (d["observed"].clone(), d["term"].clone(), d["class"].clone()),
            (
                json!("schema:OutOfStock"),
                json!("OutOfStock"),
                json!("NOT_AVAILABLE")
            )
        );
    }
}
