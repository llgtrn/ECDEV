//! Transport integrity and source-reader parity. No proof execution or canonical admission.
mod common;

use common::*;
use ecdev_governance::compact::{self, codec::Encoder, facts, history};
use ecdev_governance::digest::{content_digest, hex};
use ecdev_governance::evidence::{self, Record};
use ecdev_governance::formats::json;
use ecdev_governance::migration::source::{capture, read_export_bound, Capture};
use ecdev_governance::schema::{Ecosystem, EdgeKind, ProofKind, Scope};

fn source(name: &str) -> Repo {
    let r = extinct_baseline(name);
    history::History::record(&r.root, &history::Batch::default()).unwrap();
    std::fs::create_dir_all(r.root.join(facts::KNOWLEDGE_DIR)).unwrap();
    r
}

#[test]
fn actual_source_bytes_graph_aliases_rights_and_history_roundtrip() {
    let root = ecdev_governance::default_root();
    let baseline = read_export_bound(
        &root.join(".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json"),
        Some("ef930f0e26aab701ef384f9992d0b32abf3d101ac5bcb842ffa4d045bdce9a1d"),
    )
    .unwrap();
    assert_eq!(baseline.files.len(), 590);
    let before = capture(&root).unwrap();
    let output = before.render().unwrap();
    let read = Capture::parse(&output).unwrap();
    assert_eq!(before, read);
    assert_eq!(read.render().unwrap(), output);
    // Preserve the immutable 590-payload capture. Three exact declaration updates accompany
    // append-only evidence batches, including the two preserved failed attempts.
    assert_eq!(before.files.len(), 734);
    let mut unchanged = 0;
    for (path, bytes) in &baseline.files {
        let current = read.files.get(path).unwrap();
        let changed_digest = match path.as_str() {
            ".ecdev/declared/donors.rs" => {
                Some("sha256:be4adcb88fb07873e552d841538db5943b8e6cd775ee9929e713cc1909df73b2")
            }
            ".ecdev/declared/repository.rs" => {
                Some("sha256:865f66227754acf95a945f681339fa892fb58c032c843d9a4d64f7048f347f20")
            }
            ".ecdev/declared/technologies.rs" => {
                Some("sha256:3758d4ab8790353624999a9e39ce2e32dde394782f0501ede5b37506ae5f3ad2")
            }
            _ => None,
        };
        if let Some(digest) = changed_digest {
            assert_ne!(current, bytes);
            assert_eq!(content_digest(current), digest, "{path}");
        } else {
            assert_eq!(current, bytes, "original payload changed: {path}");
            unchanged += 1;
        }
    }
    assert_eq!(unchanged, 587);
    for (path, bytes) in &read.files {
        assert_eq!(*bytes, std::fs::read(root.join(path)).unwrap());
    }
    let declaration = read.declaration().unwrap();
    assert_eq!(declaration, ecdev_governance::declare::load(&root).unwrap());
    assert_eq!(declaration.repository.id, "ecdev");
    assert_eq!(declaration.donors.len(), 68);
    assert_eq!(declaration.repository.nodes.len(), 12);
    let mut expected = baseline.declaration().unwrap();
    let baseline_graph = ecdev_governance::graph::Graph::from_declaration(&expected).0;
    let futures = expected
        .donors
        .iter_mut()
        .find(|donor| donor.key == "dependency--github-com-rust-lang-futures-rs")
        .unwrap();
    for name in [
        "futures-channel",
        "futures-core",
        "futures-io",
        "futures-sink",
        "futures-task",
        "futures-util",
    ] {
        futures.packages.push(ecdev_governance::declare::Package {
            ecosystem: Ecosystem::Cargo,
            name: name.into(),
        });
    }
    let stream = futures
        .capabilities
        .iter_mut()
        .find(|capability| capability.key == "futures.stream-and-sink-traits")
        .unwrap();
    for locator in [
        "substrate/net/src/server.rs::streams_refuse_bytes_after_synchronized_shutdown",
        "apps/server/tests/event_stream.rs::persisted_events_replay_reconnect_and_stop_through_the_real_app",
    ] {
        stream.proofs.push(proof(ProofKind::Regression, locator));
    }
    let json = expected
        .donors
        .iter_mut()
        .find(|donor| donor.key == "dependency--github-com-serde-rs-json")
        .unwrap();
    for (key, locator) in [
        (
            "serde_json.parse-text",
            "adapter/keepa/src/client.rs::native_json_http_keeps_exact_money_raw_evidence_and_unknown_costs",
        ),
        (
            "serde_json.json-macro",
            "adapter/keepa/tests/native_json.rs::native_macro_preserves_exact_money_and_unknowns",
        ),
    ] {
        json.capabilities
            .iter_mut()
            .find(|capability| capability.key == key)
            .unwrap()
            .proofs
            .push(proof(ProofKind::Regression, locator));
    }
    let prior_graph = ecdev_governance::graph::Graph::from_declaration(&expected).0;
    assert_eq!(
        (prior_graph.nodes.len(), prior_graph.edges.len()),
        (823, 1116)
    );
    expected.repository.edges.insert(
        0,
        ecdev_governance::declare::Edge {
            from: "commerce.web-research".into(),
            to: "substrate.net".into(),
            kind: EdgeKind::DependsOn,
            scope: Scope::Runtime,
        },
    );
    let http_proofs = [
        (
            ProofKind::Regression,
            "substrate/net/tests/client.rs::response_head_arrives_before_body_and_owns_the_connection",
        ),
        (
            ProofKind::Regression,
            "substrate/net/tests/client.rs::reading_the_body_does_not_restart_the_response_deadline",
        ),
        (
            ProofKind::Parity,
            "adapter/web/src/native_http.rs::direct_http_matches_retained_transport_on_recorded_wire",
        ),
        (
            ProofKind::Regression,
            "adapter/web/src/native_http.rs::direct_http_refusals_preserve_body_and_encoding_bounds",
        ),
        (
            ProofKind::Regression,
            "adapter/web/src/native_http.rs::native_http_has_no_proxy_tls_or_credential_dispatch",
        ),
    ];
    let http = expected
        .technologies
        .iter_mut()
        .find(|technology| technology.key == "net.http-server")
        .unwrap();
    for (kind, locator) in http_proofs {
        http.proofs.push(proof(kind, locator));
    }
    assert_eq!(
        declaration, expected,
        "only six package aliases, nine proof locators and one explicit caller edge change"
    );
    let graph = ecdev_governance::graph::Graph::from_declaration(&declaration).0;
    assert_eq!((graph.nodes.len(), graph.edges.len()), (828, 1122));
    assert!(baseline_graph
        .nodes
        .keys()
        .all(|id| graph.nodes.contains_key(id)));
    assert!(baseline_graph.edges.is_subset(&graph.edges));
    assert!(prior_graph
        .nodes
        .keys()
        .all(|id| graph.nodes.contains_key(id)));
    assert!(prior_graph.edges.is_subset(&graph.edges));
    let proof_ids: std::collections::BTreeSet<_> = http_proofs
        .iter()
        .map(|(_, locator)| {
            ecdev_governance::graph::NodeId::of(
                ecdev_governance::graph::NAMESPACE,
                &ecdev_governance::graph::proof_key("ecdev", locator),
            )
        })
        .collect();
    assert_eq!(
        graph
            .nodes
            .keys()
            .filter(|id| !prior_graph.nodes.contains_key(id))
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        proof_ids
    );
    let mut added_edges: std::collections::BTreeSet<_> = proof_ids
        .iter()
        .map(|id| ecdev_governance::graph::GEdge {
            from: *id,
            to: ecdev_governance::graph::NodeId::of(
                ecdev_governance::graph::NAMESPACE,
                &ecdev_governance::graph::technology_key("net.http-server"),
            ),
            kind: EdgeKind::Verifies,
            scope: Scope::Semantic,
        })
        .collect();
    added_edges.insert(ecdev_governance::graph::GEdge {
        from: ecdev_governance::graph::NodeId::of(
            ecdev_governance::graph::NAMESPACE,
            "commerce.web-research",
        ),
        to: ecdev_governance::graph::NodeId::of(
            ecdev_governance::graph::NAMESPACE,
            "substrate.net",
        ),
        kind: EdgeKind::DependsOn,
        scope: Scope::Runtime,
    });
    assert_eq!(
        graph
            .edges
            .difference(&prior_graph.edges)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        added_edges
    );
    assert_eq!(
        json::parse(&output).unwrap().get("source_graph"),
        Some(&json::parse(&graph.to_json().render()).unwrap())
    );
    assert!(graph
        .nodes
        .keys()
        .all(|id| id.to_string().starts_with("ec1:")));
    let counts = |capture: &Capture| {
        let mut fact_ids = Vec::new();
        let mut evidence_count = 0;
        let mut retired_count = 0;
        for (path, bytes) in &capture.files {
            if path.starts_with(".ecdev/knowledge/") {
                fact_ids.extend(facts::decode(bytes).unwrap().iter().map(|f| f.id()));
            } else if path.starts_with(".ecdev/evidence/") {
                if compact::codec::Decoder::tag_of(bytes) == Some(evidence::RETIRED_TAG) {
                    retired_count += evidence::decode_retired(bytes).unwrap().len();
                } else {
                    evidence_count += 1;
                }
            }
        }
        (fact_ids, evidence_count, retired_count)
    };
    let (old_fact_ids, old_evidence, old_retired) = counts(&baseline);
    assert_eq!(
        (old_fact_ids.len(), old_evidence, old_retired),
        (1861, 582, 8)
    );
    let (fact_ids, evidence_count, retired_count) = counts(&read);
    assert_eq!(
        (fact_ids.len(), evidence_count, retired_count),
        (1861, 726, 8)
    );
    assert_eq!(fact_ids, old_fact_ids);
    assert_eq!(fact_ids[0], "f1:001be0aa9a05870b591986296071dd26");
    let appended: Vec<_> = read
        .files
        .iter()
        .filter(|(path, _)| !baseline.files.contains_key(*path))
        .collect();
    assert_eq!(appended.len(), 144);
    let json_refresh_paths = [
        ".ecdev/evidence/1102a40959cb79534b32f23bd6ede8c2.ecg",
        ".ecdev/evidence/11a890088e480d322a21c2a8a20d7397.ecg",
        ".ecdev/evidence/18dda8232b53a21b4a987528a21a712e.ecg",
        ".ecdev/evidence/1dab69108dc0960f1513c8dd83831493.ecg",
        ".ecdev/evidence/266600e7fe89707dead0a096c148ef2d.ecg",
        ".ecdev/evidence/72510c183355b8ccf3ee7ab9e2e56510.ecg",
        ".ecdev/evidence/830612fdafbe40a7bac9c78db0df7a18.ecg",
        ".ecdev/evidence/9c125781c87f8c6adcde217b5db29620.ecg",
        ".ecdev/evidence/a5a1214b9e6db9c382dfa974e4080723.ecg",
        ".ecdev/evidence/a847725e3a1ec34613bc00b8afd16f2a.ecg",
        ".ecdev/evidence/c55b4c559fb1fa0984015e1f9e796286.ecg",
        ".ecdev/evidence/ca16a48495aa89665cf0550446bcd8ce.ecg",
        ".ecdev/evidence/cc200d8ea3340d8f7c08ab6e9bf8ef4a.ecg",
        ".ecdev/evidence/d75450aeefcc261a0e074d6643821bca.ecg",
        ".ecdev/evidence/e93b24cb10aba58e3c4613d26954f130.ecg",
        ".ecdev/evidence/f440dc4f124d62e220078be7e2ecb555.ecg",
    ];
    let http_refresh_paths = [
        ".ecdev/evidence/0070f180769cd84a515fdb36f62af469.ecg",
        ".ecdev/evidence/09dedeeff2bcc6f1fe6d65536c8884fc.ecg",
        ".ecdev/evidence/0d0d01b1f753a2acad02b3eedbd06ba7.ecg",
        ".ecdev/evidence/10e4a0fe5d9e16796eb6f45688573a97.ecg",
        ".ecdev/evidence/1244dedc81401813fe72c146c4d96290.ecg",
        ".ecdev/evidence/1322fad04ed3d312d7f757e5aa54c3d8.ecg",
        ".ecdev/evidence/1495ba87c544c4e27b7c80f805ee7436.ecg",
        ".ecdev/evidence/1602eaa8396a2d5ac0d084e78dae4c64.ecg",
        ".ecdev/evidence/18a0c81a0ffb57b6d3985680c665b6ff.ecg",
        ".ecdev/evidence/1b3ecbdb225965002b1876207225ee20.ecg",
        ".ecdev/evidence/1b47f24a56ce058d43e048475f4141c9.ecg",
        ".ecdev/evidence/1e823ab9486566952a83dace83b4cc15.ecg",
        ".ecdev/evidence/25c40aabcf7e18ec10935b822da7a66a.ecg",
        ".ecdev/evidence/25c8910f334fc3e0ca494b47bc3a265c.ecg",
        ".ecdev/evidence/25fc8ffc828f1041137ff31cff124de5.ecg",
        ".ecdev/evidence/263cb2d1d02e10869e8c1eece3ab5eec.ecg",
        ".ecdev/evidence/2fac0515ef5b6714f97bcbc2628eb074.ecg",
        ".ecdev/evidence/340d57616920cca5c47a6e9f9f0cd518.ecg",
        ".ecdev/evidence/353b316cce13640cf3d640d3ae415f0d.ecg",
        ".ecdev/evidence/37fad176569f59579e704750814b90eb.ecg",
        ".ecdev/evidence/386109b50a36947211ea54fcd586d74b.ecg",
        ".ecdev/evidence/3965303d8e7b7a98c4322ee7f1fc99a6.ecg",
        ".ecdev/evidence/396d4e8ac64fdd76670697061c891340.ecg",
        ".ecdev/evidence/3b57df56326992464fdb609ea2a7b95a.ecg",
        ".ecdev/evidence/42c621ab2a620f4af94b8a9a9ed1d148.ecg",
        ".ecdev/evidence/4c476ac29e5d6b16add53f5e1927666c.ecg",
        ".ecdev/evidence/4c49c5874e12304d0560ab307c53bb2f.ecg",
        ".ecdev/evidence/4cc82b912946dd1175e89924d0cddd87.ecg",
        ".ecdev/evidence/4e6c2def75330b37830014ab2d2c5f0c.ecg",
        ".ecdev/evidence/521bb4fe463e31bb5ef8973348bdef11.ecg",
        ".ecdev/evidence/5349b2840d8196443209d8129c992ad3.ecg",
        ".ecdev/evidence/541504768a3b86df3dfaacb4c2ec96d1.ecg",
        ".ecdev/evidence/56f53b0f894f4bfdd74a1f3a27035b97.ecg",
        ".ecdev/evidence/58e08ba085911de2f9222a2837a84228.ecg",
        ".ecdev/evidence/5974eb2105e5e7c38878e4c63037646a.ecg",
        ".ecdev/evidence/5f9a716835ca9d89547d7ef6ca3dbe12.ecg",
        ".ecdev/evidence/61cb68bcc5de1478caf470e9b544f87e.ecg",
        ".ecdev/evidence/6403ebaa3499f80405f3232ebc93fb0e.ecg",
        ".ecdev/evidence/69b22d2a780e3bc6759ccaa17b206851.ecg",
        ".ecdev/evidence/6a1532574e6a2a654fcddef8419416d9.ecg",
        ".ecdev/evidence/6a8f2e9535f23887a3eea0258b51030f.ecg",
        ".ecdev/evidence/6c111982eb08cdceb9a69280a0adef59.ecg",
        ".ecdev/evidence/6ec62c31abcafbd7a8efe39546348eb4.ecg",
        ".ecdev/evidence/7224a38d0a4211675ad47acb63702185.ecg",
        ".ecdev/evidence/751e2eed25ea7093160edfc210c37f48.ecg",
        ".ecdev/evidence/7729e81bd15a7e28775c05c8c66c9d17.ecg",
        ".ecdev/evidence/793ce298cf3520eeee8bd42b7c67c038.ecg",
        ".ecdev/evidence/82962a432906fe6c97daf1cfd9378492.ecg",
        ".ecdev/evidence/84b18f8eef928d0fb8d186ca19749328.ecg",
        ".ecdev/evidence/87fbef552fd49f79f1a5642cae9f874e.ecg",
        ".ecdev/evidence/8a06dd1a7c98bdde9a6e8a3dc2bcec1e.ecg",
        ".ecdev/evidence/8f569c9055c0a61b56cb2bd4aa1c77a3.ecg",
        ".ecdev/evidence/928d83ef6d090bd637171e77cb21bd49.ecg",
        ".ecdev/evidence/9783ddc79b1d73f11b8be229b1e0114e.ecg",
        ".ecdev/evidence/a2569e740a79777809ba704ee53c8be1.ecg",
        ".ecdev/evidence/a4839f1d1bc96b010c005a10dab712b7.ecg",
        ".ecdev/evidence/a5cc31abeca1ade88c8f6acc1ec32e17.ecg",
        ".ecdev/evidence/a642e3a6ae25aba54058558a7b6f010b.ecg",
        ".ecdev/evidence/a6a9e51e6344445edd3d6f0063ae6490.ecg",
        ".ecdev/evidence/abfd216362c4402ae2203b78137780ca.ecg",
        ".ecdev/evidence/b00deae6745b70822a607d805cc85e38.ecg",
        ".ecdev/evidence/b81f0b41f24164a2fdde8c17ea50a9c7.ecg",
        ".ecdev/evidence/b83e70e8a4871140afee3891a85adc30.ecg",
        ".ecdev/evidence/bc0b50295a478e91a9dde30108466a79.ecg",
        ".ecdev/evidence/bc1b1351cc02f5248b170514a34d2f56.ecg",
        ".ecdev/evidence/c02e3eca981ae9d72397e2e62b016847.ecg",
        ".ecdev/evidence/c0927b05bbb37880ffde348b88597ef0.ecg",
        ".ecdev/evidence/c098956e512a1f4af4fd996606d5d215.ecg",
        ".ecdev/evidence/c6f5818f494c8ba5dd1705d5c54bad8d.ecg",
        ".ecdev/evidence/c93a84edcb0a584a4b533abf61bd32d5.ecg",
        ".ecdev/evidence/c959b6aea3d236131f9f3760bf5a1cde.ecg",
        ".ecdev/evidence/ca4f9eddb8c29825719a6006da432887.ecg",
        ".ecdev/evidence/ccc45db0373b182ba72d0e0e6da7cfcf.ecg",
        ".ecdev/evidence/d2052df4e1e3109fc5a8528a8586c086.ecg",
        ".ecdev/evidence/d3399ea41c9fce7fee50508f125c823b.ecg",
        ".ecdev/evidence/d47dbda0e59d4aab486b4cfe99a698d6.ecg",
        ".ecdev/evidence/d4ff513885480a9a359fefcae6169d41.ecg",
        ".ecdev/evidence/d8c4db2c311aaf350d009e7a5929980f.ecg",
        ".ecdev/evidence/d9ae501e7785740c20b2665f867e9711.ecg",
        ".ecdev/evidence/da6acdbe12f2190a40885a3c62361359.ecg",
        ".ecdev/evidence/dadc4ab8378be2607826eeb83bcfc20f.ecg",
        ".ecdev/evidence/dbe3c92ab005904ae1494aa5ea2c5fd8.ecg",
        ".ecdev/evidence/de478586556b616cbeb18aa58809061c.ecg",
        ".ecdev/evidence/e014cf8206d13e5301e35d42e24b7e04.ecg",
        ".ecdev/evidence/e89e26b8adac81fd8d97ff90e42722e8.ecg",
        ".ecdev/evidence/eb3c7bde9e2e42f53ec0833738301a41.ecg",
        ".ecdev/evidence/eb52f7d0a7ac9d9607bc48efe2523b69.ecg",
        ".ecdev/evidence/ef2241b0946a332d44dcd0fdbbe2575b.ecg",
        ".ecdev/evidence/f882d82414420a6284a5de58ef5b0b78.ecg",
        ".ecdev/evidence/fb6d4178f9db3a312be7d70a250d76a2.ecg",
        ".ecdev/evidence/fff6cc4945977e0a7a718d9cf5cf7dbd.ecg",
        ".ecdev/evidence/fff9ba3cfdefa822062cdadf11fbd9a2.ecg",
    ];
    let mut fingerprints = String::new();
    let mut json_fingerprints = String::new();
    let mut http_fingerprints = String::new();
    let mut original_count = 0;
    let mut original_passed = 0;
    let mut json_count = 0;
    let mut http_count = 0;
    let mut passed = 0;
    let mut failed = Vec::new();
    for (path, bytes) in appended {
        assert!(path.starts_with(".ecdev/evidence/"), "{path}");
        assert_eq!(
            path,
            &format!(".ecdev/evidence/{}", evidence::address(bytes))
        );
        let record = Record::decode(bytes).unwrap();
        assert_eq!(*bytes, record.encode());
        let fingerprint = format!("{path} {}\n", content_digest(bytes));
        if json_refresh_paths.contains(&path.as_str()) {
            assert!(record.passed, "JSON refresh record failed: {path}");
            json_count += 1;
            json_fingerprints.push_str(&fingerprint);
        } else if http_refresh_paths.contains(&path.as_str()) {
            assert!(record.passed, "direct HTTP refresh record failed: {path}");
            http_count += 1;
            http_fingerprints.push_str(&fingerprint);
        } else {
            original_count += 1;
            original_passed += usize::from(record.passed);
            fingerprints.push_str(&fingerprint);
        }
        if record.passed {
            passed += 1;
        } else {
            failed.push((path.as_str(), record.locator, record.kind, record.subject));
        }
    }
    // Bind each exact sorted path/raw-byte batch separately, preserving both failed attempts.
    assert_eq!(original_count, 36);
    assert_eq!(original_passed, 34);
    assert_eq!(
        content_digest(fingerprints.as_bytes()),
        "sha256:1d0ae8f97d4f91757b424dc7d1013089849f99da27648b919c41e6a023fe637d"
    );
    assert_eq!(json_count, 16);
    assert_eq!(
        content_digest(json_fingerprints.as_bytes()),
        "sha256:6c061c9f897a5f122a4cf84e5271119cf6ddb8eb9caf0f01669cc8415cca5896"
    );
    assert_eq!(http_count, 92);
    assert_eq!(
        content_digest(http_fingerprints.as_bytes()),
        "sha256:fbb47d224dcf1ec0ff08808ad1a6d76bf50b50e61624cc20a767e9f06904caf3"
    );
    assert_eq!(passed, 142);
    assert_eq!(
        failed,
        vec![
            (
                ".ecdev/evidence/506dbcaf68a1ce8754ef180090e80c68.ecg",
                "apps/server/tests/event_stream.rs::persisted_events_replay_reconnect_and_stop_through_the_real_app".into(),
                ProofKind::Regression,
                "substrate.net".into(),
            ),
            (
                ".ecdev/evidence/f727c3b006637021a6537eea6ddf3f94.ecg",
                "apps/server/tests/protocol.rs::the_console_the_api_and_mcp_over_http_conform".into(),
                ProofKind::Regression,
                "substrate.net".into(),
            ),
        ]
    );
    assert_eq!(
        capture(&root).unwrap(),
        before,
        "read-only export leaves source untouched"
    );
}

#[test]
fn cli_exports_without_assessment_or_writes_and_rejects_mutation_options() {
    let r = source("export-cli");
    let before = capture(&r.root).unwrap();
    let (code, out) = r.cli(&["migrate", "export", "--json"]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(Capture::parse(&out).unwrap(), before);
    assert!(out.contains("HEAD_OBSERVED_ONLY") && out.contains("NOT_ENROLLED"));
    assert_eq!(r.cli(&["migrate", "export", "--force"]).0, 2);
    assert_eq!(r.cli(&["migrate", "export", "--out", "x"]).0, 2);
    assert_eq!(r.cli(&["migrate", "export", "extra"]).0, 2);
    assert_eq!(capture(&r.root).unwrap(), before);
    assert!(!r.exists("x"));
    assert!(!r.exists("target"));
}

#[test]
fn captured_worktree_bytes_are_never_attested_as_observed_head() {
    let r = source("capture-head");
    r.write(".git/HEAD", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
    r.write(".git/index", "");
    let old = capture(&r.root).unwrap();
    r.write(
        ".ecdev/declared/universe/captured.jsonl",
        "{\"unknown_cost\":null}\n",
    );
    let new = capture(&r.root).unwrap();
    assert_eq!(new.observed_head, old.observed_head);
    assert_ne!(old.files, new.files);
    let out = new.render().unwrap();
    assert!(out.contains("CAPTURED_BYTES; HEAD_OBSERVED_ONLY"));
    assert_eq!(Capture::parse(&out).unwrap(), new);
}

#[test]
fn transport_rejects_tampering_duplicates_unknown_metadata_and_unsafe_paths() {
    let r = source("capture-transport");
    let original = capture(&r.root).unwrap();
    let out = original.render().unwrap();
    for bad in [
        out.replacen("ecdev-source-capture-v1", "ecdev-source-capture-v2", 1),
        out.replacen("\"format\":", "\"extra\": null, \"format\":", 1),
        out.replacen("\"format\":", "\"format\": \"old\", \"format\":", 1),
        out.replacen("\"source_id\": \"ecdev\"", "\"source_id\": \"other\"", 1),
        out.replacen("\"sha256\": \"", "\"sha256\": \"0", 1),
        out.replacen(".ecdev/declared/", ".ecdev/declared/../", 1),
    ] {
        assert!(Capture::parse(&bad).is_err());
    }
    let mut traversal = original.clone();
    traversal
        .files
        .insert(".ecdev/declared/../../escape".into(), vec![]);
    assert!(traversal.render().is_err());
    assert_eq!(capture(&r.root).unwrap(), original);
}

#[test]
fn duplicate_empty_or_reserved_source_keys_never_reduce_the_exported_graph() {
    for case in 0..7 {
        let r = source("source-key-integrity");
        r.edit(|d| match case {
            0 => d.repository.nodes.push(d.repository.nodes[0].clone()),
            1 => d.repository.nodes[0].key.clear(),
            2 => d.repository.nodes[0].key = "reserved/key".into(),
            3 => d.donors.push(d.donors[0].clone()),
            4 => d.donors[0].key.clear(),
            5 => {
                let c = d.donors[0].capabilities[0].clone();
                d.donors[0].capabilities.push(c);
            }
            _ => d.donors[0].capabilities[0].key.clear(),
        });
        assert!(capture(&r.root).is_err(), "case {case}");
    }
}

#[test]
fn oversized_files_and_recursive_inputs_fail_with_a_reader_error() {
    let r = source("source-budget");
    let file = std::fs::File::create(r.root.join(".ecdev/declared/large.rs")).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    assert!(capture(&r.root).unwrap_err().contains("budget"));
    assert!(
        Capture::parse(&format!("{}0{}", "[".repeat(10_000), "]".repeat(10_000)))
            .unwrap_err()
            .contains("nesting")
    );
    r.remove(".ecdev/declared/large.rs");
    r.write(
        ".ecdev/declared/donors.rs",
        &format!("{}0{}", "[".repeat(10_000), "]".repeat(10_000)),
    );
    assert!(capture(&r.root).is_err());
}

#[test]
fn overflowing_history_sequences_and_knowledge_counts_cannot_wrap_to_zero() {
    let r = source("source-integer-overflow");
    let mut history_bytes = Encoder::new(history::HISTORY_TAG);
    history_bytes.strs(&[]).strs(&[]).strs(&[]).u64(1);
    history_bytes.bytes.extend_from_slice(&[128; 9]);
    history_bytes.bytes.push(2);
    history_bytes.str("").u64(0).u64(0);
    let path =
        compact::write_addressed(&r.root, history::HISTORY_DIR, &history_bytes.finish()).unwrap();
    assert!(capture(&r.root).unwrap_err().contains("integer overflow"));
    r.remove(&path);
    let mut knowledge = Encoder::new(facts::KNOWLEDGE_TAG).finish();
    knowledge.extend_from_slice(&[128; 9]);
    knowledge.push(2);
    compact::write_addressed(&r.root, facts::KNOWLEDGE_DIR, &knowledge).unwrap();
    assert!(capture(&r.root).unwrap_err().contains("integer overflow"));
}

#[test]
fn incomplete_or_corrupt_inputs_fail_instead_of_shrinking_denominators() {
    for dir in ["declared", "history", "evidence", "knowledge"] {
        let r = source("missing-source-dir");
        r.remove(&format!(".ecdev/{dir}"));
        assert!(capture(&r.root).is_err(), "missing {dir}");
    }
    let r = source("empty-history");
    r.remove(".ecdev/history");
    std::fs::create_dir(r.root.join(".ecdev/history")).unwrap();
    assert!(capture(&r.root).unwrap_err().contains("history is missing"));
    let r = source("bad-address");
    r.write(".ecdev/history/wrong.ecg", "not an addressed record");
    assert!(capture(&r.root).unwrap_err().contains("content address"));
    r.remove(".ecdev/history/wrong.ecg");
    for bytes in [
        b"ECG\x02\x0b".to_vec(),
        Encoder::new(99).finish(),
        [history::Batch::default().encode(), vec![0]].concat(),
    ] {
        let path = compact::write_addressed(&r.root, history::HISTORY_DIR, &bytes).unwrap();
        assert!(capture(&r.root).is_err());
        r.remove(&path);
    }
}

#[test]
fn malformed_evidence_booleans_are_rejected_even_under_a_valid_new_address() {
    let r = source("bad-boolean");
    let record = Record {
        locator: "x".into(),
        kind: ProofKind::Regression,
        passed: true,
        proof_digest: "unknown".into(),
        subject: "x".into(),
        subject_digest: "unknown".into(),
        command: "never executed".into(),
    };
    let mut bytes = record.encode();
    // ECG envelope, one-byte locator length, 'x', proof-kind rank, then bool.
    bytes[8] = 2;
    assert!(Record::decode(&bytes).is_err());
    compact::write_addressed(&r.root, evidence::EVIDENCE_DIR, &bytes).unwrap();
    assert!(capture(&r.root).unwrap_err().contains("invalid boolean"));
}

#[test]
fn legacy_history_tags_keep_original_bytes_and_unknown_capability_floors() {
    for tag in [history::HISTORY_TAG_V1, history::HISTORY_TAG_V2] {
        let r = source("legacy-history-capture");
        r.remove(history::HISTORY_DIR);
        let mut encoder = Encoder::new(tag);
        encoder.strs(&["old-donor".into()]).strs(&[]).u64(0);
        let bytes = encoder.finish();
        let path = compact::write_addressed(&r.root, history::HISTORY_DIR, &bytes).unwrap();
        let c = capture(&r.root).unwrap();
        assert_eq!(c.files[&path], bytes);
        assert_eq!(
            Capture::parse(&c.render().unwrap()).unwrap().files[&path],
            bytes
        );
        assert!(hex(&bytes).starts_with("45434701"));
    }
}

#[test]
fn valid_source_record_order_and_nonminimal_integer_bytes_are_never_normalized() {
    let r = source("source-encoded-bytes");
    let record = Record {
        locator: "x".into(),
        kind: ProofKind::Regression,
        passed: false,
        proof_digest: "unknown".into(),
        subject: "x".into(),
        subject_digest: "unknown".into(),
        command: "captured only".into(),
    };
    let mut retired = Encoder::new(evidence::RETIRED_TAG);
    retired.u64(2);
    // The native reader supports both rows even though a new writer would sort them.
    for seq in [2, 1] {
        retired
            .u64(seq)
            .str("")
            .str("original-address.ecg")
            .bytes(&record.encode());
    }
    let retired_bytes = retired.finish();
    let retired_path =
        compact::write_addressed(&r.root, evidence::EVIDENCE_DIR, &retired_bytes).unwrap();
    let mut active_bytes = record.encode();
    active_bytes.splice(5..6, [0x81, 0]); // valid two-byte LEB128 encoding of locator length 1
    assert_eq!(Record::decode(&active_bytes).unwrap(), record);
    let active_path =
        compact::write_addressed(&r.root, evidence::EVIDENCE_DIR, &active_bytes).unwrap();
    let c = capture(&r.root).unwrap();
    let read = Capture::parse(&c.render().unwrap()).unwrap();
    assert_eq!(read.files[&retired_path], retired_bytes);
    assert_eq!(read.files[&active_path], active_bytes);
}

#[cfg(unix)]
#[test]
fn source_control_directories_and_files_must_not_be_symlinks() {
    for path in [
        ".ecdev/knowledge",
        ".ecdev/declared/repository.rs",
        ".ecdev",
    ] {
        let r = source("source-link");
        let backup = r.root.join("held");
        std::fs::rename(r.root.join(path), &backup).unwrap();
        std::os::unix::fs::symlink(&backup, r.root.join(path)).unwrap();
        assert!(capture(&r.root).is_err(), "{path}");
    }
}
