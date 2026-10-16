//! Transport integrity and source-reader parity. No proof execution or canonical admission.
mod common;

use common::*;
use ecdev_governance::compact::{self, codec::Encoder, facts, history};
use ecdev_governance::digest::hex;
use ecdev_governance::evidence::{self, Record};
use ecdev_governance::formats::json;
use ecdev_governance::migration::source::{capture, Capture};
use ecdev_governance::schema::ProofKind;

fn source(name: &str) -> Repo {
    let r = extinct_baseline(name);
    history::History::record(&r.root, &history::Batch::default()).unwrap();
    std::fs::create_dir_all(r.root.join(facts::KNOWLEDGE_DIR)).unwrap();
    r
}

#[test]
fn actual_source_bytes_graph_aliases_rights_and_history_roundtrip() {
    let root = ecdev_governance::default_root();
    let before = capture(&root).unwrap();
    let output = before.render().unwrap();
    let read = Capture::parse(&output).unwrap();
    assert_eq!(before, read);
    assert_eq!(read.render().unwrap(), output);
    // The original 299-file projection plus 291 appended, freshly executed proof records.
    assert_eq!(before.files.len(), 590);
    for (path, bytes) in &read.files {
        assert_eq!(*bytes, std::fs::read(root.join(path)).unwrap());
    }
    let declaration = read.declaration().unwrap();
    assert_eq!(declaration, ecdev_governance::declare::load(&root).unwrap());
    assert_eq!(declaration.repository.id, "ecdev");
    assert_eq!(declaration.donors.len(), 68);
    assert_eq!(declaration.repository.nodes.len(), 12);
    let graph = ecdev_governance::graph::Graph::from_declaration(&declaration).0;
    assert_eq!(
        json::parse(&output).unwrap().get("source_graph"),
        Some(&json::parse(&graph.to_json().render()).unwrap())
    );
    assert!(graph
        .nodes
        .keys()
        .all(|id| id.to_string().starts_with("ec1:")));
    let mut fact_ids = Vec::new();
    let mut evidence_count = 0;
    let mut retired_count = 0;
    for (path, bytes) in &read.files {
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
    assert_eq!(
        (fact_ids.len(), evidence_count, retired_count),
        (1861, 582, 8)
    );
    assert_eq!(fact_ids[0], "f1:001be0aa9a05870b591986296071dd26");
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
