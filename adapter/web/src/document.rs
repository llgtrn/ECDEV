//! Declared HTML decoding. Wire bytes remain the capture; decoding is a projection.
use ecdev_core::sha256::Sha256;
use encoding_rs::{Encoding, UTF_8};
use scraper::{Html, Selector};
use serde_json::{Value, json};

fn charset(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['\'', '"']).to_string())
    })
}

pub fn decode(bytes: &[u8], content_type: &str) -> Result<(String, Value), String> {
    if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        return Err("UNSUPPORTED_DECLARED_CHARSET".into());
    }
    let (encoding, skip, source, declared) =
        if let Some((encoding, skip)) = Encoding::for_bom(bytes) {
            (encoding, skip, "BOM", None)
        } else {
            let header = charset(content_type);
            // HTML's declaration prescan is bounded. DOM parsing excludes comments and script text.
            let prefix: String = bytes
                .iter()
                .take(1024)
                .map(|&b| if b.is_ascii() { char::from(b) } else { '?' })
                .collect();
            let doc = Html::parse_document(&prefix);
            let meta = doc
                .select(&Selector::parse("meta").unwrap())
                .find_map(|node| {
                    node.value()
                        .attr("charset")
                        .map(str::to_string)
                        .or_else(|| {
                            node.value()
                                .attr("http-equiv")
                                .filter(|v| v.eq_ignore_ascii_case("content-type"))?;
                            charset(node.value().attr("content")?)
                        })
                });
            let source = if header.is_some() {
                "HTTP_CONTENT_TYPE"
            } else if meta.is_some() {
                "HTML_META"
            } else {
                "VALID_UTF8_DEFAULT"
            };
            let label = header.or(meta);
            let encoding = match &label {
                Some(label) => {
                    Encoding::for_label(label.as_bytes()).ok_or("UNSUPPORTED_DECLARED_CHARSET")?
                }
                None => UTF_8,
            };
            (encoding, 0, source, label)
        };
    let (text, errors) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if errors {
        return Err("INVALID_DOCUMENT_ENCODING".into());
    }
    let recipe = json!({"encoding":encoding.name(),"source":source,"declared_label":declared,
        "bom_bytes_removed":skip,"replacement_characters_inserted":false,
        "wire_capture_sha256":format!("{:x}",Sha256::digest(bytes)),
        "decoded_utf8_sha256":format!("{:x}",Sha256::digest(text.as_bytes())),
        "policy":"BOM, HTTP charset, first 1024 bytes HTML meta, then strict UTF-8; no statistical guessing"});
    Ok((text.into_owned(), recipe))
}

/// All extracted evidence points to original bytes, including non-UTF-8 captures.
pub fn extract(bytes: &[u8], content_type: &str, source: &str) -> Result<Value, String> {
    let (text, recipe) = decode(bytes, content_type)?;
    let mut result = crate::extract_with_hash(
        &text,
        source,
        recipe["wire_capture_sha256"].as_str().unwrap(),
    )?;
    result["document_decoding"] = recipe;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_w3lib_declared_document_oracle() {
        let oracle: Value =
            serde_json::from_str(include_str!("../tests/fixtures/w3lib-document.json")).unwrap();
        assert_eq!(
            oracle["commit_sha"],
            "537c5d46455ae8b2c67b53fc03b36ef1da8c4837"
        );
        assert_eq!(oracle["cases"].as_array().unwrap().len(), 54);
        for case in oracle["cases"].as_array().unwrap() {
            let bytes: Vec<u8> = case["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            let (text, _) = decode(&bytes, case["content_type"].as_str().unwrap()).unwrap();
            assert_eq!(text, case["expected"], "{}", case["name"]);
        }
    }
    #[test]
    fn declared_japanese_preserves_wire_provenance() {
        let html = r#"<meta charset="Shift_JIS"><script type="application/ld+json">{"@type":"Product","name":"耐熱ガラス","content_hash":"source assertion","offers":{"price":"2980","priceCurrency":"JPY"}}</script>"#;
        let (bytes, _, errors) = encoding_rs::SHIFT_JIS.encode(html);
        assert!(!errors);
        let value = extract(&bytes, "text/html", "https://shop.example/item").unwrap();
        assert_eq!(value["products"][0]["title"], "耐熱ガラス");
        assert_eq!(value["products"][0]["price_minor"], 2980);
        assert_eq!(
            value["products"][0]["raw_json_ld"]["content_hash"],
            "source assertion"
        );
        assert_eq!(
            value["structured_data"][0]["value"]["content_hash"],
            "source assertion"
        );
        let wire = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(value["content_hash"], wire);
        assert_eq!(
            value["products"][0]["provenance"]["raw_capture_sha256"],
            wire
        );
        assert_ne!(value["document_decoding"]["decoded_utf8_sha256"], wire);
        assert_eq!(value["document_decoding"]["source"], "HTML_META");
    }
    #[test]
    fn precedence_and_failure_are_explicit() {
        assert_eq!(
            decode(b"\xef\xbb\xbfhello", "text/html; charset=unknown")
                .unwrap()
                .0,
            "hello"
        );
        assert_eq!(
            decode(b"<meta charset='unknown'>", "text/html; charset=utf-8")
                .unwrap()
                .1["source"],
            "HTTP_CONTENT_TYPE"
        );
        assert!(decode(&[0x81], "text/html; charset=shift_jis").is_err());
        assert!(decode(&[0xff], "text/html").is_err());
        assert_eq!(
            decode(b"hello", "text/html; charset=unknown").unwrap_err(),
            "UNSUPPORTED_DECLARED_CHARSET"
        );
        assert_eq!(
            decode(
                b"<script>\"<meta charset='unknown'>\"</script>",
                "text/html"
            )
            .unwrap()
            .1["source"],
            "VALID_UTF8_DEFAULT"
        );
    }
}
