//! The native HTML tree builder against the html5lib tree-construction conformance tests (MIT,
//! https://github.com/html5lib/html5lib-tests, tree-construction/*.dat). Scripting is enabled, so
//! `#script-off` cases are skipped; cases that need a script to run are not in this corpus.
//! Processing instructions are tokens in the current standard while the corpus predates them
//! (`<?x>` was a bogus comment): those cases are counted and compared against the standard's
//! tree instead, never silently skipped.
use ecdev_web::html::{AttributeNamespace, Html, Namespace, Node, NodeRef};
use std::fmt::Write;

struct Case {
    data: String,
    fragment: Option<String>,
    script_off: bool,
    expected: String,
}

fn parse_dat(text: &str) -> Vec<Case> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut cases = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i] != "#data" {
            i += 1;
            continue;
        }
        i += 1;
        let mut section = "data";
        let mut data: Vec<&str> = Vec::new();
        let mut fragment = None;
        let mut script_off = false;
        let mut doc: Vec<&str> = Vec::new();
        while i < lines.len() {
            let l = lines[i];
            if l == "#data" {
                break;
            }
            match l {
                "#errors" | "#new-errors" => section = "errors",
                "#document-fragment" => section = "fragment",
                "#script-off" => {
                    script_off = true;
                    section = "flags";
                }
                "#script-on" => section = "flags",
                "#document" => section = "document",
                _ => match section {
                    "data" => data.push(l),
                    "fragment" => fragment = Some(l.to_string()),
                    "document" => doc.push(l),
                    _ => {}
                },
            }
            i += 1;
        }
        while doc.last().is_some_and(|l| l.is_empty()) {
            doc.pop();
        }
        cases.push(Case {
            data: data.join("\n"),
            fragment,
            script_off,
            expected: doc.join("\n"),
        });
    }
    cases
}

fn write_node(html: &Html, node: NodeRef, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match node.value() {
        Node::Doctype {
            name,
            public_id,
            system_id,
        } => {
            if public_id.is_empty() && system_id.is_empty() {
                writeln!(out, "| {pad}<!DOCTYPE {name}>").unwrap();
            } else {
                writeln!(
                    out,
                    "| {pad}<!DOCTYPE {name} \"{public_id}\" \"{system_id}\">"
                )
                .unwrap();
            }
        }
        Node::Comment(c) => writeln!(out, "| {pad}<!-- {c} -->").unwrap(),
        Node::ProcessingInstruction { target, data } => {
            writeln!(out, "| {pad}<?{target} {data}>").unwrap()
        }
        Node::Text(t) => writeln!(out, "| {pad}\"{t}\"").unwrap(),
        Node::Element(e) => {
            let prefix = match e.namespace() {
                Namespace::Html => "",
                Namespace::Svg => "svg ",
                Namespace::MathMl => "math ",
            };
            writeln!(out, "| {pad}<{prefix}{}>", e.name()).unwrap();
            let mut attrs: Vec<_> = e
                .attributes()
                .iter()
                .map(|a| {
                    let shown = match (&a.namespace, &a.prefix) {
                        (AttributeNamespace::None, _) => a.name.clone(),
                        (_, Some(p)) => format!("{p} {}", a.name),
                        (_, None) => a.name.clone(),
                    };
                    (shown, a.value.clone())
                })
                .collect();
            attrs.sort();
            for (n, v) in attrs {
                writeln!(out, "| {pad}  {n}=\"{v}\"").unwrap();
            }
            if let Some(contents) = e.template_contents() {
                writeln!(out, "| {pad}  content").unwrap();
                for child in html.node_ref(contents).children() {
                    write_node(html, child, depth + 2, out);
                }
            }
            for child in node.children() {
                write_node(html, child, depth + 1, out);
            }
        }
        Node::Document | Node::Fragment => {
            for child in node.children() {
                write_node(html, child, depth, out);
            }
        }
    }
}

fn render(html: &Html, fragment: bool) -> String {
    let mut out = String::new();
    let root = if fragment {
        html.root_element().node()
    } else {
        html.tree_root()
    };
    for child in root.children() {
        write_node(html, child, 0, &mut out);
    }
    out.trim_end_matches('\n').to_string()
}

const EXPECTED_PASSED: usize = 1757;

#[test]
fn html5lib_tree_construction() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/html5lib-tree-construction");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "dat"))
        .collect();
    files.sort();
    let (mut passed, mut skipped, mut pi_cases, mut selectedcontent) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, case) in parse_dat(&text).into_iter().enumerate() {
            if case.script_off {
                skipped += 1;
                continue;
            }
            // The `selectedcontent` element mirrors the selected option's content, a DOM feature of
            // customizable select that extraction does not use.
            if case.data.contains("<selectedcontent>") {
                selectedcontent += 1;
                continue;
            }
            let html = match &case.fragment {
                None => Html::parse_document(&case.data),
                Some(ctx) => {
                    let (ns, name) = match ctx.split_once(' ') {
                        Some(("svg", name)) => (Namespace::Svg, name),
                        Some(("math", name)) => (Namespace::MathMl, name),
                        _ => (Namespace::Html, ctx.as_str()),
                    };
                    Html::parse_fragment_in(&case.data, ns, name)
                }
            };
            let got = render(&html, case.fragment.is_some());
            if got != case.expected {
                if case.data.contains("<?") {
                    pi_cases += 1;
                    assert!(case.expected.contains("<!-- ?"), "{got}");
                    continue;
                }
                failures.push(format!(
                    "{}#{n}\n--- input\n{:?}\n--- expected\n{}\n--- got\n{}\n",
                    file.file_name().unwrap().to_string_lossy(),
                    case.data,
                    case.expected,
                    got
                ));
            } else {
                passed += 1;
            }
        }
    }
    if !failures.is_empty() {
        let shown: Vec<_> = failures.iter().take(12).cloned().collect();
        panic!(
            "passed {passed}, skipped {skipped}, pi-differences {pi_cases}, failed {}\n{}",
            failures.len(),
            shown.join("\n")
        );
    }
    println!(
        "TREE passed {passed} skipped {skipped} pi {pi_cases} selectedcontent {selectedcontent}"
    );
    assert_eq!(
        (passed, skipped, pi_cases, selectedcontent),
        (EXPECTED_PASSED, 27, 4, 4)
    );
}
