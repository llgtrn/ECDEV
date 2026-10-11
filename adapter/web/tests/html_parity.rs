//! The native HTML parser and selectors against frozen results of the retired scraper/html5ever
//! donor and of Chromium (see `fixtures/html-differential-digests.json`): trees, selector hits
//! and text over the html5lib documents and a generated misnested-markup corpus.
//!
//! Documents where the native parser deliberately differs are listed in the fixture, never
//! skipped silently: processing instructions are tokens (the donor makes comments), the select
//! element parses its content per the current standard (the donor predates it), MathML
//! `annotation-xml` integration points are honoured, active formatting elements are
//! reconstructed before `<math>`/`<svg>`, and Chromium reads CDATA in a MathML text integration
//! point as a bogus comment where the standard defines a CDATA section.
mod html_corpus;
use ecdev_web::html::Html;
use html_corpus::*;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/html-differential-digests.json")).unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn check(section: &Value, docs: Vec<(String, String)>) {
    let excluded = strings(&section["excluded_tree_differences"]);
    let text_excluded = strings(&section["excluded_text_templates"]);
    let selectors_excluded = strings(&section["excluded_donor_tree_inconsistent"]);
    assert_eq!(section["documents"].as_u64().unwrap() as usize, docs.len());
    let (mut tree, mut selectors, mut text) = (Vec::new(), Vec::new(), Vec::new());
    for (id, data) in &docs {
        if excluded.contains(id) {
            continue;
        }
        let html = Html::parse_document(data);
        tree.push((id.clone(), render(&html)));
        if !selectors_excluded.contains(id) {
            selectors.push((id.clone(), selector_signature(&html)));
        }
        if !text_excluded.contains(id) && !selectors_excluded.contains(id) {
            text.push((id.clone(), text_signature(&html)));
        }
    }
    assert_eq!(tree.len() as u64, section["compared"].as_u64().unwrap());
    let d = |v: &Vec<(String, String)>| digest(v.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    assert_eq!(d(&tree), section["tree_digest"], "tree");
    assert_eq!(d(&selectors), section["selector_digest"], "selectors");
    assert_eq!(d(&text), section["text_digest"], "text");
}

#[test]
fn html5lib_documents_match_the_donor() {
    check(&fixture()["html5lib"], html5lib_documents());
}

#[test]
fn generated_documents_match_the_donor() {
    let docs = generated_documents()
        .into_iter()
        .enumerate()
        .map(|(i, d)| (i.to_string(), d))
        .collect();
    check(&fixture()["generated"], docs);
}

#[test]
fn generated_documents_match_chromium() {
    let f = fixture();
    let section = &f["chromium_generated"];
    let excluded = strings(&section["excluded_cdata_in_text_integration_point"]);
    let docs = generated_documents();
    assert_eq!(section["documents"].as_u64().unwrap() as usize, docs.len());
    let mut tree = Vec::new();
    for (i, data) in docs.iter().enumerate() {
        let id = i.to_string();
        if excluded.contains(&id) {
            assert!(data.contains("<![CDATA["), "{id}");
            continue;
        }
        tree.push((id, render(&Html::parse_document(data))));
    }
    assert_eq!(tree.len() as u64, section["compared"].as_u64().unwrap());
    let d = digest(tree.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    assert_eq!(d, section["tree_digest"]);
}

#[test]
fn fragments_match_the_donor() {
    let f = fixture();
    let section = &f["fragments"];
    let excluded = strings(&section["excluded"]);
    let mut docs: Vec<(String, String)> = html5lib_documents();
    docs.extend(
        generated_documents()
            .into_iter()
            .take(10000)
            .enumerate()
            .map(|(i, d)| (format!("g{i}"), d)),
    );
    assert_eq!(section["documents"].as_u64().unwrap() as usize, docs.len());
    let (mut tree, mut text) = (Vec::new(), Vec::new());
    for (id, data) in &docs {
        if excluded.contains(id) {
            continue;
        }
        let html = Html::parse_fragment(data);
        tree.push((id.clone(), render_fragment(&html)));
        text.push((id.clone(), text_signature(&html)));
    }
    assert_eq!(tree.len() as u64, section["compared"].as_u64().unwrap());
    let d = |v: &Vec<(String, String)>| digest(v.iter().map(|(a, b)| (a.as_str(), b.as_str())));
    assert_eq!(d(&tree), section["tree_digest"], "tree");
    assert_eq!(d(&text), section["text_digest"], "text");
}
