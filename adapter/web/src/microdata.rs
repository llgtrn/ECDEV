//! Native nested HTML microdata graph; independently compiled, donor used only as an oracle.
//! Behavior derived from extruct's BSD-3-Clause parser. Notice: tests/fixtures/extruct-LICENSE.txt.
use scraper::{ElementRef, Html, Selector};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub fn extract(doc: &Html, base: Option<&Url>, strict: bool) -> Value {
    let scopes: Vec<_> = doc
        .select(&Selector::parse("[itemscope]").unwrap())
        .collect();
    let scope_ids: BTreeMap<_, _> = scopes
        .iter()
        .enumerate()
        .map(|(i, n)| (format!("{:?}", n.id()), i + 1))
        .collect();
    let ids: BTreeMap<_, _> = doc
        .select(&Selector::parse("[id]").unwrap())
        .filter_map(|n| n.value().attr("id").map(|id| (id.to_string(), n)))
        .fold(BTreeMap::new(), |mut m, (k, v)| {
            m.entry(k).or_insert(v);
            m
        });
    let mut parser = Parser {
        base,
        strict,
        scope_ids,
        ids,
        seen: BTreeSet::new(),
    };
    let items: Vec<_> = scopes
        .into_iter()
        .filter_map(|node| parser.item(node, 0))
        .collect();
    json!(items)
}
struct Parser<'a, 'b> {
    base: Option<&'b Url>,
    strict: bool,
    scope_ids: BTreeMap<String, usize>,
    ids: BTreeMap<String, ElementRef<'a>>,
    seen: BTreeSet<usize>,
}
impl Parser<'_, '_> {
    fn scope_id(&self, node: ElementRef<'_>) -> Option<usize> {
        self.scope_ids.get(&format!("{:?}", node.id())).copied()
    }
    fn parent_scope(&self, node: ElementRef<'_>) -> Option<usize> {
        node.ancestors()
            .filter_map(ElementRef::wrap)
            .find_map(|n| self.scope_id(n))
    }
    fn item(&mut self, node: ElementRef<'_>, depth: usize) -> Option<Value> {
        let id = self.scope_id(node)?;
        if depth > 64 || !self.seen.insert(id) {
            return None;
        }
        let mut item = serde_json::Map::new();
        let types: Vec<_> = node
            .value()
            .attr("itemtype")
            .unwrap_or("")
            .split_whitespace()
            .collect();
        if !types.is_empty() {
            item.insert(
                "type".into(),
                if types.len() == 1 && !self.strict {
                    json!(types[0])
                } else {
                    json!(types)
                },
            );
            if let Some(value) = node
                .value()
                .attr("itemid")
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                item.insert("id".into(), json!(value));
            }
        }
        let mut properties: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for p in node
            .select(&Selector::parse("[itemprop]").unwrap())
            .filter(|p| p.id() != node.id())
        {
            if self.parent_scope(p) == Some(id) {
                self.property(p, depth, &mut properties);
            }
        }
        for reference in node
            .value()
            .attr("itemref")
            .unwrap_or("")
            .split_whitespace()
        {
            let Some(referenced) = self.ids.get(reference).copied() else {
                continue;
            };
            if referenced.value().attr("itemscope").is_some()
                && referenced.value().attr("itemprop").is_some()
            {
                self.property(referenced, depth, &mut properties);
            } else {
                let base_scope = self
                    .scope_id(referenced)
                    .or_else(|| self.parent_scope(referenced));
                let mut props = vec![];
                if referenced.value().attr("itemprop").is_some() {
                    props.push(referenced);
                }
                props.extend(
                    referenced
                        .select(&Selector::parse("[itemprop]").unwrap())
                        .filter(|p| p.id() != referenced.id()),
                );
                for p in props {
                    if self.parent_scope(p) == base_scope {
                        self.property(p, depth, &mut properties);
                    }
                }
            }
        }
        if properties.is_empty() {
            item.insert("value".into(), self.value(node, depth, true));
        } else {
            item.insert(
                "properties".into(),
                Value::Object(
                    properties
                        .into_iter()
                        .map(|(key, mut values)| {
                            (
                                key,
                                if !self.strict && values.len() == 1 {
                                    values.pop().unwrap()
                                } else {
                                    json!(values)
                                },
                            )
                        })
                        .collect(),
                ),
            );
        }
        Some(Value::Object(item))
    }
    fn property(
        &mut self,
        node: ElementRef<'_>,
        depth: usize,
        out: &mut BTreeMap<String, Vec<Value>>,
    ) {
        let value = self.value(node, depth, false);
        for name in node
            .value()
            .attr("itemprop")
            .unwrap_or("")
            .split_whitespace()
        {
            out.entry(name.into()).or_default().push(value.clone());
        }
    }
    fn value(&mut self, node: ElementRef<'_>, depth: usize, force: bool) -> Value {
        if !force && node.value().attr("itemscope").is_some() {
            return self.item(node, depth + 1).unwrap_or(Value::Null);
        }
        let attr = match node.value().name() {
            "meta" => Some("content"),
            "audio" | "embed" | "iframe" | "img" | "source" | "track" | "video" => Some("src"),
            "a" | "area" | "link" => Some("href"),
            "object" => Some("data"),
            "data" | "meter" => Some("value"),
            "time" => Some("datetime"),
            _ => None,
        };
        if let Some(attr) = attr {
            let value = node.value().attr(attr).unwrap_or("");
            if matches!(attr, "src" | "href" | "data") {
                let value = value.trim_matches([' ', '\t', '\n', '\r', '\u{c}']);
                if value.split_once(':').is_some_and(|(scheme, _)| {
                    !scheme.is_empty()
                        && scheme
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                }) {
                    return json!(value);
                }
                return json!(
                    self.base
                        .and_then(|base| base.join(value).ok())
                        .map(|u| u.to_string())
                        .unwrap_or(value.into())
                );
            }
            return json!(value);
        }
        if let Some(content) = node.value().attr("content").filter(|s| !s.is_empty()) {
            return json!(content);
        }
        if node
            .value()
            .attr("itemprop")
            .is_some_and(|p| p.ends_with("-input") || p.ends_with("-output"))
        {
            let mut value = serde_json::Map::new();
            if node.value().attr("required").is_some() {
                value.insert("valueRequired".into(), json!(true));
            }
            if let Some(name) = node.value().attr("name").filter(|s| !s.is_empty()) {
                value.insert("valueName".into(), json!(name));
            }
            return Value::Object(value);
        }
        let mut text = Text {
            out: String::new(),
            prev: None,
            breaks: 2,
        };
        text.walk(node, 0);
        json!(text.out.trim())
    }
}
struct Text {
    out: String,
    prev: Option<String>,
    breaks: u8,
}
impl Text {
    fn newline(&mut self, tag: &str) {
        let count = if [
            "blockquote",
            "dl",
            "figure",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "ol",
            "p",
            "pre",
            "title",
            "ul",
        ]
        .contains(&tag)
        {
            2
        } else if [
            "article",
            "aside",
            "br",
            "dd",
            "details",
            "div",
            "dt",
            "fieldset",
            "figcaption",
            "footer",
            "form",
            "header",
            "hr",
            "legend",
            "li",
            "main",
            "nav",
            "table",
            "tr",
        ]
        .contains(&tag)
        {
            1
        } else {
            0
        };
        if count > self.breaks {
            for _ in self.breaks..count {
                self.out.push('\n');
            }
            self.breaks = count;
            self.prev = None;
        }
    }
    fn text(&mut self, raw: &str) {
        let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return;
        }
        if let Some(prev) = &self.prev {
            let punct = text.starts_with([',', ':', ';', '.', '!', '?', '"', ')', ']']);
            if prev.ends_with(char::is_whitespace) || (!punct && !prev.ends_with('(')) {
                self.out.push(' ');
            }
        }
        self.out.push_str(&text);
        self.prev = Some(raw.into());
        self.breaks = 0;
    }
    fn walk(&mut self, node: ElementRef<'_>, depth: usize) {
        if depth > 256 || ["script", "style", "link", "meta"].contains(&node.value().name()) {
            return;
        }
        self.newline(node.value().name());
        for child in node.children() {
            if let Some(t) = child.value().as_text() {
                self.text(t);
            } else if let Some(el) = ElementRef::wrap(child) {
                self.walk(el, depth + 1);
            }
        }
        self.newline(node.value().name());
    }
}
/// Retain a mapping from normalized commerce paths to the captured raw microdata graph.
pub fn normalize(items: &Value) -> (Value, BTreeMap<String, String>) {
    fn convert(v: &Value, raw: &str, path: &str, map: &mut BTreeMap<String, String>) -> Value {
        map.insert(path.into(), raw.into());
        match v {
            Value::Array(a) => json!(
                a.iter()
                    .enumerate()
                    .map(|(i, v)| convert(v, &format!("{raw}/{i}"), &format!("{path}/{i}"), map))
                    .collect::<Vec<_>>()
            ),
            Value::Object(o) if o.contains_key("properties") || o.contains_key("type") => {
                let mut out = serde_json::Map::new();
                if let Some(types) = o.get("type") {
                    let local = |s: &str| s.rsplit(['/', '#']).next().unwrap_or(s).to_string();
                    out.insert(
                        "@type".into(),
                        match types {
                            Value::String(s) => json!(local(s)),
                            Value::Array(a) => json!(
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(local)
                                    .collect::<Vec<_>>()
                            ),
                            _ => Value::Null,
                        },
                    );
                }
                if let Some(id) = o.get("id") {
                    out.insert("@id".into(), id.clone());
                }
                if let Some(properties) = o.get("properties").and_then(Value::as_object) {
                    for (k, v) in properties {
                        let escaped = k.replace('~', "~0").replace('/', "~1");
                        out.insert(
                            k.clone(),
                            convert(
                                v,
                                &format!("{raw}/properties/{escaped}"),
                                &format!("{path}/{escaped}"),
                                map,
                            ),
                        );
                    }
                }
                Value::Object(out)
            }
            _ => v.clone(),
        }
    }
    let mut paths = BTreeMap::new();
    let normalized = convert(items, "", "", &mut paths);
    (normalized, paths)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locked_extruct_microdata_oracle() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/extruct-microdata.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let doc = Html::parse_document(case["html"].as_str().unwrap());
            let base = case["base_url"].as_str().and_then(|s| Url::parse(s).ok());
            assert_eq!(
                extract(&doc, base.as_ref(), case["strict"].as_bool().unwrap()),
                case["expected"],
                "case {}",
                case["name"]
            );
        }
    }
}
