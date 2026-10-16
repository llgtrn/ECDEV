//! Transport integrity and source-reader parity. No proof execution or canonical admission.
mod common;

use common::*;
use ecdev_governance::compact::{self, codec::Encoder, facts, history};
use ecdev_governance::digest::{content_digest, hex};
use ecdev_governance::evidence::{self, Record};
use ecdev_governance::formats::json;
use ecdev_governance::migration::source::{capture, read_export_bound, Capture};
use ecdev_governance::schema::{Ecosystem, ProofKind};

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
    // Preserve the immutable 590-payload capture. Only the donor declaration changes;
    // the refresh appends evidence, including the two failed attempts, without replacing it.
    assert_eq!(before.files.len(), 642);
    let mut unchanged = 0;
    for (path, bytes) in &baseline.files {
        let current = read.files.get(path).unwrap();
        if path == ".ecdev/declared/donors.rs" {
            assert_ne!(current, bytes);
            assert_eq!(
                content_digest(current),
                "sha256:be4adcb88fb07873e552d841538db5943b8e6cd775ee9929e713cc1909df73b2"
            );
        } else {
            assert_eq!(current, bytes, "original payload changed: {path}");
            unchanged += 1;
        }
    }
    assert_eq!(unchanged, 589);
    for (path, bytes) in &read.files {
        assert_eq!(*bytes, std::fs::read(root.join(path)).unwrap());
    }
    let declaration = read.declaration().unwrap();
    assert_eq!(declaration, ecdev_governance::declare::load(&root).unwrap());
    assert_eq!(declaration.repository.id, "ecdev");
    assert_eq!(declaration.donors.len(), 68);
    assert_eq!(declaration.repository.nodes.len(), 12);
    let mut expected = baseline.declaration().unwrap();
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
    assert_eq!(
        declaration, expected,
        "only six package aliases and four proof locators change"
    );
    let graph = ecdev_governance::graph::Graph::from_declaration(&declaration).0;
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
        (1861, 634, 8)
    );
    assert_eq!(fact_ids, old_fact_ids);
    assert_eq!(fact_ids[0], "f1:001be0aa9a05870b591986296071dd26");
    let appended: Vec<_> = read
        .files
        .iter()
        .filter(|(path, _)| !baseline.files.contains_key(*path))
        .collect();
    assert_eq!(appended.len(), 52);
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
    let mut fingerprints = String::new();
    let mut json_fingerprints = String::new();
    let mut original_count = 0;
    let mut original_passed = 0;
    let mut json_count = 0;
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
    assert_eq!(passed, 50);
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
