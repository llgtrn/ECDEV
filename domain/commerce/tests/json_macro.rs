//! The `json!` macro on the shapes the code base writes, against the text serde_json's macro gave
//! for the same invocations (frozen when the donor was retired).
use ecdev_core::json;

#[test]
fn macro_invocations_build_the_frozen_values() {
    let x = 3;
    let y = 4u8;
    let s = String::from("str");
    let v = vec![1u8, 2, 3];
    let b = true;
    let mut m = std::collections::BTreeMap::new();
    m.insert("k".to_string(), 1u8);
    let key = "dyn";
    let _ = (&x, &y, &s, &v, &b, &m, &key);
    let cases: Vec<(&str, ecdev_core::json::Value)> = vec![
        ("null", json!(null)),
        ("true", json!(true)),
        ("false", json!(false)),
        ("1", json!(1)),
        ("-1", json!(-1)),
        ("1.5", json!(1.5)),
        ("\"s\"", json!("s")),
        ("[]", json!([])),
        ("{}", json!({})),
        ("[1,2,3]", json!([1, 2, 3])),
        ("[1,2,3]", json!([1, 2, 3,])),
        ("[null,true,false]", json!([null, true, false])),
        ("{\"a\":1}", json!({"a":1})),
        ("{\"a\":1}", json!({"a":1,})),
        (
            "{\"a\":{\"b\":[1,{\"c\":null}]}}",
            json!({"a":{"b":[1,{"c":null}]}}),
        ),
        (
            "{\"a\":3,\"b\":[3,4],\"c\":{\"d\":3}}",
            json!({"a":x,"b":[x,y],"c":{"d":x}}),
        ),
        ("[3,4,\"z\"]", json!([x, y, "z"])),
        ("{\"j\":8,\"k\":4}", json!({"k":x+1,"j":y*2})),
        ("{\"s\":\"3-4\"}", json!({"s":format!("{}-{}",x,y)})),
        ("{\"v\":[1,2,3]}", json!({"v":vec![1,2,3]})),
        ("{\"n\":null,\"o\":5}", json!({"o":Some(5),"n":None::<u32>})),
        ("{\"dyn\":3}", json!({(key):x})),
        ("{\"k3\":4}", json!({(format!("k{}",x)):y})),
        ("[{\"a\":1},{\"a\":2}]", json!([{"a":1},{"a":2}])),
        ("[[1],[2,[3]]]", json!([[1], [2, [3]]])),
        ("{\"a\":[],\"b\":{}}", json!({"a":[],"b":{}})),
        (
            "{\"float\":-1250.0,\"neg\":-5}",
            json!({"neg":-5,"float":-1.25e3}),
        ),
        (
            "{\"f\":false,\"n\":null,\"t\":true}",
            json!({"t":true,"f":false,"n":null}),
        ),
        (
            "{\"slice\":[1,2,3],\"text\":\"str\",\"vec\":[1,2,3]}",
            json!({"text":s.clone(),"slice":&v[..],"vec":v.clone()}),
        ),
        (
            "{\"nested\":{\"deep\":{\"deeper\":[1,2,{\"x\":\"y\"}]}}}",
            json!({"nested":{"deep":{"deeper":[1,2,{"x":"y"}]}}}),
        ),
        (
            "{\"inner\":{\"inner\":1}}",
            json!({"inner":json!({"inner":1})}),
        ),
        ("[1,\"a\"]", json!([json!(1), json!("a")])),
        ("{\"a\":1.0}", json!({"a":x as f64 / 3.0})),
        (
            "{\"i\":-9223372036854775808,\"u\":18446744073709551615}",
            json!({"u":18446744073709551615u64,"i":-9223372036854775808i64}),
        ),
        (
            "{\"e\":1e+300,\"small\":0.1}",
            json!({"e":1e300,"small":0.1}),
        ),
        ("{\"c\":\"a\"}", json!({"c":'a'})),
        ("{\"b\":true}", json!({"b":b})),
        ("{\"tuple\":[1,\"a\"]}", json!({"tuple":(1,"a")})),
        ("{\"map\":{\"k\":1}}", json!({"map":m.clone()})),
        ("{\"unit\":null}", json!({"unit":()})),
        ("{\"x\":3,\"y\":4}", json!({"x":x,"y":y,})),
        ("[1,[2,[3,[4]]]]", json!([1, [2, [3, [4]]]])),
        (
            "{\"quote\":\"a\\\"b\\\\c\\n\"}",
            json!({"quote":"a\"b\\c\n"}),
        ),
        ("{\"uni\":\"日本😀\"}", json!({"uni":"日本😀"})),
    ];
    for (expected, value) in cases {
        assert_eq!(value.to_string(), expected);
    }
}
