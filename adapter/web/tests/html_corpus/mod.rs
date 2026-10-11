//! Shared corpora and signatures for the HTML parity proofs: the html5lib document tests, a
//! deterministic generated corpus of misnested markup, and canonical renderings of a parsed tree.
#![allow(dead_code)]
use ecdev_core::sha256::Sha256;
use ecdev_web::html::{ElementRef, Html, Namespace, Node, NodeRef, Selector};
use std::fmt::Write;

pub const GENERATED_DOCUMENTS: usize = 30000;
pub const GENERATOR_SEED: u64 = 0x9E3779B97F4A7C15;

/// Document (non-fragment) tests of the html5lib tree-construction corpus: (id, input).
pub fn html5lib_documents() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/html5lib-tree-construction");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "dat"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        let mut n = 0;
        let mut it = text.split('\n').peekable();
        while let Some(l) = it.next() {
            if l != "#data" {
                continue;
            }
            let mut data = Vec::new();
            while let Some(&l2) = it.peek() {
                if l2.starts_with('#') && l2 != "#data" {
                    break;
                }
                data.push(l2);
                it.next();
            }
            let mut fragment = false;
            for l3 in it.clone() {
                if l3 == "#document-fragment" {
                    fragment = true;
                }
                if l3 == "#document" || l3 == "#data" {
                    break;
                }
            }
            if !fragment {
                out.push((
                    format!("{}#{n}", file.file_name().unwrap().to_string_lossy()),
                    data.join("\n"),
                ));
            }
            n += 1;
        }
    }
    out
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[(self.next() % v.len() as u64) as usize]
    }
}

pub fn generated_documents() -> Vec<String> {
    let open = [
        "<html>",
        "<head>",
        "<body>",
        "<div>",
        "<p>",
        "<span>",
        "<a href=x>",
        "<b>",
        "<i>",
        "<em>",
        "<strong>",
        "<font color=r>",
        "<nobr>",
        "<ul>",
        "<ol>",
        "<li>",
        "<dl>",
        "<dd>",
        "<dt>",
        "<table>",
        "<tbody>",
        "<thead>",
        "<tr>",
        "<td>",
        "<th>",
        "<caption>",
        "<colgroup>",
        "<col>",
        "<form>",
        "<input>",
        "<input type=hidden>",
        "<button>",
        "<h1>",
        "<h2>",
        "<pre>",
        "<textarea>",
        "<title>",
        "<style>",
        "<script>",
        "<noscript>",
        "<iframe>",
        "<xmp>",
        "<plaintext>",
        "<img src=a>",
        "<br>",
        "<hr>",
        "<svg>",
        "<g>",
        "<path d=1>",
        "<foreignObject>",
        "<math>",
        "<mi>",
        "<mo>",
        "<mtext>",
        "<center>",
        "<marquee>",
        "<object>",
        "<applet>",
        "<frameset>",
        "<frame>",
        "<base href=/>",
        "<meta charset=x>",
        "<link rel=a>",
        "<ruby>",
        "<rt>",
        "<rp>",
        "<h3>",
        "<main>",
        "<section>",
        "<article>",
        "<nav>",
        "<details>",
        "<summary>",
        "<address>",
        "<blockquote>",
    ];
    let close = [
        "</html>",
        "</head>",
        "</body>",
        "</div>",
        "</p>",
        "</span>",
        "</a>",
        "</b>",
        "</i>",
        "</em>",
        "</strong>",
        "</font>",
        "</nobr>",
        "</ul>",
        "</ol>",
        "</li>",
        "</dl>",
        "</dd>",
        "</dt>",
        "</table>",
        "</tbody>",
        "</thead>",
        "</tr>",
        "</td>",
        "</th>",
        "</caption>",
        "</colgroup>",
        "</form>",
        "</button>",
        "</h1>",
        "</h2>",
        "</pre>",
        "</textarea>",
        "</title>",
        "</style>",
        "</script>",
        "</noscript>",
        "</svg>",
        "</g>",
        "</math>",
        "</mi>",
        "</center>",
        "</marquee>",
        "</object>",
        "</br>",
        "</sarcasm>",
        "</frameset>",
        "</ruby>",
        "</h3>",
        "</main>",
        "</section>",
    ];
    let text = [
        "x", " ", "\n", "hello", "&amp;", "&lt;", "a b", "&nbsp;", "&#65;", "&bogus;",
    ];
    let comments = ["<!-- c -->", "<!DOCTYPE html>", "<![CDATA[x]]>", "<!>"];
    let mut rng = Rng(GENERATOR_SEED);
    let mut out = Vec::with_capacity(GENERATED_DOCUMENTS);
    for _ in 0..GENERATED_DOCUMENTS {
        let mut doc = String::new();
        let len = 3 + rng.next() % 40;
        for _ in 0..len {
            match rng.next() % 10 {
                0..=4 => doc.push_str(rng.pick(&open)),
                5..=6 => doc.push_str(rng.pick(&close)),
                7..=8 => doc.push_str(rng.pick(&text)),
                _ => doc.push_str(rng.pick(&comments)),
            }
        }
        out.push(doc);
    }
    out
}

fn walk(node: NodeRef, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match node.value() {
        Node::Document | Node::Fragment => {
            for c in node.children() {
                walk(c, depth, out);
            }
        }
        Node::Doctype {
            name,
            public_id,
            system_id,
        } => writeln!(
            out,
            "{pad}<!DOCTYPE {name} \"{public_id}\" \"{system_id}\">"
        )
        .unwrap(),
        Node::Comment(c) => writeln!(out, "{pad}<!-- {c} -->").unwrap(),
        Node::Text(t) => writeln!(out, "{pad}\"{t}\"").unwrap(),
        Node::ProcessingInstruction { target, data } => {
            writeln!(out, "{pad}<?{target} {data}>").unwrap()
        }
        Node::Element(e) => {
            let tag = match e.namespace() {
                Namespace::Html => "",
                Namespace::Svg => "svg ",
                Namespace::MathMl => "math ",
            };
            writeln!(out, "{pad}<{tag}{}>", e.name()).unwrap();
            let mut attrs: Vec<(String, String)> = e
                .attributes()
                .iter()
                .map(|a| (a.name.clone(), a.value.clone()))
                .collect();
            attrs.sort();
            for (k, v) in attrs {
                writeln!(out, "{pad}  {k}=\"{v}\"").unwrap();
            }
            for c in node.children() {
                walk(c, depth + 1, out);
            }
        }
    }
}

/// One line per node, children indented; attributes sorted; foreign elements prefixed.
pub fn render(html: &Html) -> String {
    let mut out = String::new();
    walk(html.tree_root(), 0, &mut out);
    out
}

/// The children of the root element, which is what a fragment parse produces.
pub fn render_fragment(html: &Html) -> String {
    let mut out = String::new();
    for c in html.root_element().children() {
        walk(c, 0, &mut out);
    }
    out
}

pub const SELECTORS: &[&str] = &[
    "meta",
    "title",
    "a[href]",
    "base[href]",
    "form[action]",
    "script[type='application/ld+json']",
    "meta[property],meta[name]",
    "link[rel]",
    "h1",
    "p,li,dd,td,h1,h2,h3",
    "main, [role=main]",
    "[itemscope]",
    "[id]",
    "[itemprop]",
    "div > p",
    "div p",
    "p + p",
    "p ~ p",
    ".a",
    "#a",
    "[class~=a]",
    "[lang|=en]",
    "[href^=http]",
    "[href$='.pdf']",
    "[href*=x]",
    "[type=TEXT i]",
    "input[type=checkbox]",
    "*",
    "table td",
    "TD",
    "[HREF]",
    "svg",
    "svg *",
    "tr > td",
    "body > *",
    "li a",
    "a b",
    "b a",
    "div div div",
];

/// Document-order positions of the elements matching each selector, as one line per selector.
pub fn selector_signature(html: &Html) -> String {
    let mut index = std::collections::HashMap::new();
    let mut stack = vec![html.tree_root()];
    let mut count = 0usize;
    while let Some(node) = stack.pop() {
        if ElementRef::wrap(node).is_some() {
            index.insert(node.id(), count);
            count += 1;
        }
        let mut kids: Vec<_> = node.children().collect();
        kids.reverse();
        stack.extend(kids);
    }
    let mut out = String::new();
    for sel in SELECTORS {
        let selector = Selector::parse(sel).unwrap();
        let mut hits: Vec<usize> = html.select(&selector).map(|e| index[&e.id()]).collect();
        hits.sort_unstable();
        writeln!(out, "{sel} {hits:?}").unwrap();
    }
    out
}

pub fn text_signature(html: &Html) -> String {
    html.root_element().text().collect()
}

pub fn digest<'a>(parts: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut h = Sha256::new();
    for (id, body) in parts {
        h.update(id.as_bytes());
        h.update(b"\n");
        h.update(body.as_bytes());
        h.update(b"\n\x00\n");
    }
    format!("{:x}", h.finalize())
}
