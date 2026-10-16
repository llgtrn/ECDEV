//! Portable source contract and real read-only caller. Fixture verdicts are never executed.
mod common;

use common::*;
use ecdev_governance::compact::{self, codec::Encoder, facts, history};
use ecdev_governance::declare::{Knowledge, Relevance, ResearchScope};
use ecdev_governance::digest::content_digest;
use ecdev_governance::formats::{json, Value};
use ecdev_governance::migration::{source, source_view};
use ecdev_governance::schema::FactKind;
use std::process::Command;

const FIXTURE: &str = include_str!("fixtures/source-capture-v1.json");
const META: &str = include_str!("fixtures/source-capture-v1.meta.json");

fn git(r: &Repo, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(r.path())
        .args([
            "-c",
            "user.name=source-fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}

fn source_fixture(name: &str) -> Repo {
    let r = extinct_baseline(name);
    history::History::record(r.path(), &history::Batch::default()).unwrap();
    let fact = facts::Fact::new(
        FactKind::LegacyClaim,
        "source",
        "cost",
        "UNKNOWN",
        "captured source",
        u64::MAX,
    );
    compact::write_addressed(r.path(), facts::KNOWLEDGE_DIR, &facts::encode(&[fact])).unwrap();
    r.write(
        ".ecdev/declared/universe/unknown.json",
        "{\"cost\":null,\"paid_ceiling\":17,\"effects\":\"UNKNOWN\"}\n",
    );
    r
}

#[test]
fn pinned_fixture_runs_portably_without_a_checkout_or_authority() {
    let r = Repo::new("portable-source");
    r.write("capture.json", FIXTURE);
    let capture = source::read_export(&r.root.join("capture.json")).unwrap();
    assert_eq!(capture.render().unwrap(), FIXTURE);
    let meta = json::parse(META).unwrap();
    assert_eq!(
        content_digest(FIXTURE.as_bytes()),
        "sha256:c751083d31323cf190a687452233277459d84967a6d5daf04bffb830f03c654d"
    );
    assert_eq!(
        capture.observed_head,
        "247fffae286a5b05c8677d00fa0c0a64f49dc5df"
    );
    assert_eq!(
        meta.str("source_capture_sha256"),
        Some(&content_digest(FIXTURE.as_bytes())[7..])
    );
    assert_eq!(capture.observed_head, meta.text("source_commit"));
    assert_eq!(capture.files.len(), 590);
    assert_eq!(
        capture.files.values().map(Vec::len).sum::<usize>(),
        1_105_906
    );
    let path = r.root.join("capture.json").display().to_string();
    let first = r.cli(&["migrate", "inspect-export", "--from", &path, "--json"]);
    assert_eq!(first.0, 0, "{}", first.1);
    assert_eq!(
        first,
        r.cli(&["migrate", "inspect-export", "--from", &path, "--json"])
    );
    let view = json::parse(&first.1).unwrap();
    assert_eq!(view.text("canonical_enrollment"), "BLOCKED");
    assert_eq!(view.get("canonical_identity_mapping"), Some(&Value::Null));
    assert_eq!(view.get("norl_relevance_mapping"), Some(&Value::Null));
    assert_eq!(view.items_of("donors").len(), 68);
    assert_eq!(view.items_of("facts").len(), 1861);
    assert_eq!(view.get("active_evidence_records"), Some(&Value::Int(582)));
    assert_eq!(view.get("retired_evidence_records"), Some(&Value::Int(8)));
    assert_eq!(
        view.get("source_graph"),
        json::parse(FIXTURE).unwrap().get("source_graph")
    );
    assert!(!r.exists(".ecdev") && !r.exists("target") && !r.exists(".git"));
    assert_eq!(r.read("capture.json"), FIXTURE);
}

#[test]
fn portable_caller_rejects_malformed_tampered_duplicate_and_mutation_inputs() {
    let r = Repo::new("source-caller-negatives");
    let path = r.root.join("capture.json").display().to_string();
    for bad in [
        "{broken".into(),
        FIXTURE.replacen("\"source_id\": \"ecdev\"", "\"source_id\": \"other\"", 1),
        FIXTURE.replacen("\"format\":", "\"format\": \"duplicate\", \"format\":", 1),
        FIXTURE.replacen("\"path\": \".ecdev/", "\"path\": \"../.ecdev/", 1),
        FIXTURE.replacen("\"sha256\": \"", "\"sha256\": \"00", 1),
        format!("{FIXTURE}trailing"),
    ] {
        assert_ne!(bad, FIXTURE, "mutation must hit the fixture");
        r.write("capture.json", &bad);
        assert_eq!(r.cli(&["migrate", "inspect-export", "--from", &path]).0, 2);
        assert_eq!(r.read("capture.json"), bad);
    }
    r.write("capture.json", FIXTURE);
    for extra in [
        vec!["--force"],
        vec!["--out", "output"],
        vec!["--from", &path],
        vec!["unexpected"],
    ] {
        let mut args = vec!["migrate", "inspect-export", "--from", &path];
        args.extend(extra);
        assert_eq!(r.cli(&args).0, 2);
    }
    assert!(source::read_export(r.path()).is_err());
    let large = r.root.join("oversize.json");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(64 * 1024 * 1024 * 3 + 1)
        .unwrap();
    assert!(source::read_export(&large).unwrap_err().contains("budget"));
    std::fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(source::read_export(std::path::Path::new(&path))
        .unwrap_err()
        .contains("UTF-8"));
    assert!(!r.exists("output") && !r.exists(".ecdev"));
}

#[test]
fn recorded_schema_is_portable_metadata_while_registry_digest_binds_every_byte() {
    let r = Repo::new("portable-schema-claim");
    let capture = source::Capture::parse(FIXTURE).unwrap();
    assert_ne!(
        capture.source_schema,
        ecdev_governance::protocol::schema_identity()
    );
    assert_eq!(capture.render().unwrap(), FIXTURE);
    let digest = &content_digest(FIXTURE.as_bytes())[7..];
    r.write("capture.json", FIXTURE);
    let path = r.root.join("capture.json");
    assert_eq!(
        source::read_export_bound(&path, Some(digest)).unwrap(),
        capture
    );
    let mut changed = capture.clone();
    changed.source_schema = format!("sha256:{}", "a".repeat(64));
    let changed_bytes = changed.render().unwrap();
    assert_eq!(source::Capture::parse(&changed_bytes).unwrap(), changed);
    r.write("capture.json", &changed_bytes);
    assert!(source::read_export_bound(&path, Some(digest))
        .unwrap_err()
        .contains("caller-bound"));
    let caller_path = path.display().to_string();
    assert_eq!(
        r.cli(&[
            "migrate",
            "inspect-export",
            "--from",
            &caller_path,
            "--expect-sha256",
            digest
        ])
        .0,
        2
    );
    r.write("capture.json", FIXTURE);
    assert_eq!(
        r.cli(&[
            "migrate",
            "inspect-export",
            "--from",
            &caller_path,
            "--expect-sha256",
            digest
        ])
        .0,
        0
    );
    assert!(source::read_export_bound(&path, Some("not a digest")).is_err());
    for bad in [
        FIXTURE.replacen("\"schema\":", "\"schema\": \"duplicate\", \"schema\":", 1),
        FIXTURE.replacen("ec1:", "ec1:0", 1),
        FIXTURE.replacen(&capture.source_schema, "sha256:bad", 1),
        FIXTURE.replacen("\"schema\":", "\"missing_schema\":", 1),
    ] {
        assert!(source::Capture::parse(&bad).is_err());
    }
    changed.source_schema = "malformed".into();
    assert!(changed.render().is_err());
}

#[test]
fn adoption_knowledge_absorption_aliases_and_unknowns_remain_independent() {
    let r = source_fixture("separate-source-semantics");
    r.edit(|d| {
        let mut other = d.donors[0].clone();
        other.key = "same-origin-alias".into();
        d.donors[0].capabilities[0].relevance =
            Relevance::NotAdopted("No current runtime caller".into());
        d.donors[0].capabilities[0].required = false;
        d.donors[0].capabilities[0].knowledge =
            Knowledge::StudyCandidate("Compare retained algorithm".into());
        d.donors[0].capabilities[0].absorbs = vec!["old".into()];
        other.capabilities[0].knowledge =
            Knowledge::NoResearchValue(ResearchScope::Assets, "Only vendor logos".into());
        let mut label_only = other.capabilities[0].clone();
        label_only.key = "absorbed-label-only".into();
        label_only.knowledge = Knowledge::Absorbed("Recorded study relation only".into());
        other.capabilities.push(label_only);
        d.donors.push(other);
    });
    let batch = history::Batch {
        registered: ["vanished".into()].into(),
        capabilities: [
            "geo/old".into(),
            "same-origin-alias/old".into(),
            "vanished/old".into(),
        ]
        .into(),
        ..Default::default()
    };
    history::History::record(r.path(), &batch).unwrap();
    let capture = source::capture(r.path()).unwrap();
    let view = json::parse(&source_view::inspect(&capture).unwrap().render()).unwrap();
    let donors = view.items_of("donors");
    assert_eq!(donors.len(), 2);
    assert_eq!(donors[0].get("source_id"), donors[1].get("source_id"));
    assert_ne!(donors[0].get("source_alias"), donors[1].get("source_alias"));
    let cap = &donors[0].items_of("capabilities")[0];
    assert_eq!(
        cap.at("adoption.variant").and_then(Value::as_str),
        Some("NotAdopted")
    );
    assert_eq!(
        cap.at("knowledge.variant").and_then(Value::as_str),
        Some("StudyCandidate")
    );
    assert_eq!(cap.strings("absorbs"), ["old"]);
    assert_eq!(
        donors[1].items_of("capabilities")[0]
            .at("knowledge.scope")
            .and_then(Value::as_str),
        Some("ASSETS")
    );
    assert_eq!(
        donors[1].items_of("capabilities")[1]
            .at("knowledge.variant")
            .and_then(Value::as_str),
        Some("Absorbed")
    );
    assert!(donors[1].items_of("capabilities")[1]
        .items_of("absorbs")
        .is_empty());
    let known = view.at("history.known_capabilities").unwrap().items();
    assert!(!known.contains(&Value::Str("geo/old".into())));
    assert!(known.contains(&Value::Str("same-origin-alias/old".into())));
    assert!(known.contains(&Value::Str("vanished/old".into())));
    let fact = &view.items_of("facts")[0];
    assert!(fact.text("source_id").starts_with("f1:"));
    assert_eq!(fact.text("sequence_decimal"), u64::MAX.to_string());
    assert_eq!(fact.text("value"), "UNKNOWN");
    assert_eq!(
        capture.files[".ecdev/declared/universe/unknown.json"],
        r.read(".ecdev/declared/universe/unknown.json").as_bytes()
    );
}

#[test]
fn legacy_capability_floor_is_unknown_and_unicode_origins_do_not_panic() {
    let r = source_fixture("legacy-source-floor");
    r.remove(history::HISTORY_DIR);
    let mut e = Encoder::new(history::HISTORY_TAG_V2);
    e.strs(&[]).strs(&[]).u64(0);
    compact::write_addressed(r.path(), history::HISTORY_DIR, &e.finish()).unwrap();
    r.edit(|d| d.donors[0].origin = "a€€".into());
    let view = json::parse(
        &source_view::inspect(&source::capture(r.path()).unwrap())
            .unwrap()
            .render(),
    )
    .unwrap();
    assert_eq!(view.at("history.known_capabilities"), Some(&Value::Null));
    assert_eq!(
        view.items_of("donors")[0].get("global_origin_key"),
        Some(&Value::Null)
    );
}

#[test]
fn committed_export_ignores_dirty_index_untracked_files_and_replace_refs() {
    let r = source_fixture("committed-source");
    git(&r, &["init", "-q", "--template="]);
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "source fixture"]);
    let before = source::capture_committed(r.path()).unwrap();
    assert_eq!(before, source::capture(r.path()).unwrap());
    r.write(".ecdev/declared/universe/unknown.json", "{\"cost\":0}\n");
    git(&r, &["add", ".ecdev/declared/universe/unknown.json"]);
    r.write(".ecdev/declared/repository.rs", "not a declaration");
    r.write(".ecdev/evidence/untracked.ecg", "not evidence");
    assert_eq!(source::capture_committed(r.path()).unwrap(), before);
    let tree = git(&r, &["write-tree"]);
    let replacement = git(&r, &["commit-tree", &tree, "-m", "replacement fixture"]);
    git(&r, &["replace", &before.observed_head, &replacement]);
    assert_eq!(
        git(&r, &["show", "HEAD:.ecdev/declared/universe/unknown.json"]),
        "{\"cost\":0}"
    );
    assert_eq!(source::capture_committed(r.path()).unwrap(), before);
    let (code, output) = r.cli(&["migrate", "export", "--committed", "--json"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(source::Capture::parse(&output).unwrap(), before);
    assert_eq!(
        r.cli(&["migrate", "export", "--committed", "--committed"])
            .0,
        2
    );
    assert_eq!(r.read(".ecdev/declared/repository.rs"), "not a declaration");
}

#[test]
fn committed_export_refuses_missing_controls_and_links() {
    let r = source_fixture("bad-committed-source");
    git(&r, &["init", "-q", "--template="]);
    r.remove(facts::KNOWLEDGE_DIR);
    git(&r, &["add", "."]);
    git(&r, &["commit", "-qm", "incomplete fixture"]);
    assert!(source::capture_committed(r.path())
        .unwrap_err()
        .contains("knowledge"));
    #[cfg(unix)]
    {
        std::fs::create_dir_all(r.root.join(facts::KNOWLEDGE_DIR)).unwrap();
        std::os::unix::fs::symlink(
            "../declared/repository.rs",
            r.root.join(".ecdev/knowledge/link.ecg"),
        )
        .unwrap();
        git(&r, &["add", "."]);
        git(&r, &["commit", "-qm", "linked fixture"]);
        assert!(source::capture_committed(r.path())
            .unwrap_err()
            .contains("unsupported"));
    }
}
