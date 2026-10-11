//! The native URL parser's outputs over the shared corpus equal the digests recorded while it was
//! proven identical to the url crate 2.5.8 (documented differences aside). The url crate is no longer
//! a dependency of ECDEV's own code.
use ecdev_core::sha256::Sha256;
use ecdev_core::web_address::Url;

mod web_address_corpus;
use web_address_corpus::*;

fn frozen() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/web-address-differential-digests.json"
    ))
    .unwrap()
}

#[test]
fn reference_cases_reproduce_the_frozen_digest() {
    let mut h = Sha256::new();
    for (input, base) in reference_cases() {
        h.update(native_outcome(&input, base.as_deref()));
    }
    assert_eq!(
        format!("{:x}", h.finalize()),
        frozen()["references"].as_str().unwrap()
    );
}

#[test]
fn international_hosts_reproduce_the_frozen_digest() {
    let mut h = Sha256::new();
    for host in international_hosts() {
        let parsed = Url::parse(&format!("https://{host}/"));
        h.update(parsed.as_ref().map(|u| u.as_str()).unwrap_or("ERR"));
    }
    assert_eq!(
        format!("{:x}", h.finalize()),
        frozen()["international_hosts"].as_str().unwrap()
    );
}

#[test]
fn mutator_scenarios_reproduce_the_frozen_digest() {
    let mut h = Sha256::new();
    for (base, ops) in mutator_scenarios() {
        let mut u = Url::parse(base).unwrap();
        for op in &ops {
            match op {
                Op::SetQuery(q) => u.set_query(Some(q)),
                Op::SetFragment(f) => u.set_fragment(Some(f)),
                Op::SetPath(p) => u.set_path(p),
                Op::SetUsername(n) => {
                    let _ = u.set_username(n);
                }
                Op::ClearQuery => u.set_query(None),
                Op::ClearFragment => u.set_fragment(None),
                Op::AppendPairs(n) => {
                    let mut q = u.query_pairs_mut();
                    for (k, v) in &PAIRS[..*n] {
                        q.append_pair(k, v);
                    }
                }
                Op::ClearPairs => {
                    u.query_pairs_mut().clear();
                }
                Op::Segments => {
                    if let Ok(mut s) = u.path_segments_mut() {
                        s.pop_if_empty().push("new seg").push("..").push("é");
                    }
                }
            }
            h.update(u.as_str());
        }
    }
    assert_eq!(
        format!("{:x}", h.finalize()),
        frozen()["mutators"].as_str().unwrap()
    );
}
