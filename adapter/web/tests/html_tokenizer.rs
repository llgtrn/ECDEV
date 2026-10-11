//! The native HTML tokenizer against the html5lib tokenizer conformance tests (MIT,
//! https://github.com/html5lib/html5lib-tests, tokenizer/*.test, commit c777c40).
use ecdev_web::html::tokenizer::{State, Token, Tokenizer};
use serde_json::{Value, json};

fn unescape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'u') && i + 6 <= chars.len() {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                // A surrogate half has no char; the test data pairs them, which JSON already joined.
                out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                i += 6;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn unescape_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(unescape(s)),
        Value::Array(a) => Value::Array(a.iter().map(unescape_value).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (unescape(k), unescape_value(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn state_of(name: &str) -> State {
    match name {
        "Data state" => State::Data,
        "PLAINTEXT state" => State::Plaintext,
        "RCDATA state" => State::Rcdata,
        "RAWTEXT state" => State::Rawtext,
        "Script data state" => State::ScriptData,
        "CDATA section state" => State::CdataSection,
        other => panic!("state {other}"),
    }
}

fn run(input: &str, state: State, last_start_tag: Option<&str>) -> Vec<Value> {
    let mut t = Tokenizer::new(input);
    t.state = state;
    if let Some(name) = last_start_tag {
        t.set_last_start_tag(name);
    }
    let mut out: Vec<Value> = vec![];
    let mut text = String::new();
    loop {
        let token = t.next_token();
        if let Token::Character(c) = token {
            text.push(c);
            continue;
        }
        if !text.is_empty() {
            out.push(json!(["Character", std::mem::take(&mut text)]));
        }
        match token {
            Token::Eof => break,
            Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => out.push(json!([
                "DOCTYPE",
                name,
                public_id,
                system_id,
                !force_quirks
            ])),
            Token::StartTag {
                name,
                attrs,
                self_closing,
            } => {
                let map: serde_json::Map<String, Value> =
                    attrs.into_iter().map(|(k, v)| (k, json!(v))).collect();
                if self_closing {
                    out.push(json!(["StartTag", name, map, true]));
                } else {
                    out.push(json!(["StartTag", name, map]));
                }
            }
            Token::EndTag { name, .. } => out.push(json!(["EndTag", name])),
            Token::Comment(c) => out.push(json!(["Comment", c])),
            Token::ProcessingInstruction { target, data } => {
                out.push(json!(["ProcessingInstruction", target, data]))
            }
            Token::Character(_) => unreachable!(),
        }
    }
    out
}

#[test]
fn html5lib_tokenizer_tests() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/html5lib-tokenizer");
    let (mut passed, mut failed) = (0, vec![]);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "test"))
        .collect();
    files.sort();
    for file in files {
        // xmlViolation.test checks coercion to an XML infoset, which is outside HTML parsing.
        if file.file_name().is_some_and(|n| n == "xmlViolation.test") {
            continue;
        }
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let tests = data
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .as_array()
            .unwrap();
        for test in tests {
            let double = test["doubleEscaped"].as_bool().unwrap_or(false);
            let input = test["input"].as_str().unwrap();
            let input = if double {
                unescape(input)
            } else {
                input.to_string()
            };
            let mut expected = test["output"].as_array().unwrap().clone();
            if double {
                expected = expected.iter().map(unescape_value).collect();
            }
            // Adjacent character tokens are one run in the expected output; the tokenizer emits one per character.
            let states: Vec<State> = match test["initialStates"].as_array() {
                Some(a) => a.iter().map(|s| state_of(s.as_str().unwrap())).collect(),
                None => vec![State::Data],
            };
            for state in states {
                let got = run(&input, state, test["lastStartTag"].as_str());
                if got == expected {
                    passed += 1;
                } else {
                    failed.push(format!("{} :: {} ({state:?})\n  input {input:?}\n  want {expected:?}\n  got  {got:?}", file.file_name().unwrap().to_string_lossy(), test["description"]));
                }
            }
        }
    }
    for f in failed.iter().take(25) {
        println!("FAIL {f}");
    }
    println!("TOKENIZER passed {passed} failed {}", failed.len());
    assert!(failed.is_empty());
}
