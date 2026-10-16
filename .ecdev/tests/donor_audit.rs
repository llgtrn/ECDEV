//! Descriptive source audit boundaries; fixture proof records never execute donor code.
mod common;

use common::*;
use ecdev_governance::compact::{facts, history};
use ecdev_governance::declare::{Donor, Package};
use ecdev_governance::digest::content_digest;
use ecdev_governance::donors::audit;
use ecdev_governance::formats::{json, Value};
use ecdev_governance::schema::{DonorState, Ecosystem, ExceptionKind, FactKind, UniverseKind};
use ecdev_governance::universe::import;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

fn report(r: &Repo) -> Value {
    json::parse(&audit::run(r.path()).unwrap().render()).unwrap()
}

fn donor_row<'a>(report: &'a Value, alias: &str) -> &'a Value {
    report
        .items_of("donors")
        .iter()
        .find(|d| d.str("source_alias") == Some(alias))
        .unwrap()
}

fn input<'a>(report: &'a Value, path: &str) -> &'a Value {
    report
        .at("snapshot.inputs")
        .unwrap()
        .items()
        .iter()
        .find(|d| d.str("path") == Some(path))
        .unwrap()
}

fn integer(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Int(n)) => *n,
        other => panic!("expected a measured integer: {other:?}"),
    }
}

fn unknown_oss(report: &Value) {
    assert_eq!(
        report.at("audited_oss.status"),
        Some(&Value::Str("UNKNOWN".into()))
    );
    assert_eq!(report.at("audited_oss.total"), Some(&Value::Null));
    assert_eq!(
        report.at("audited_oss.verified_extinct"),
        Some(&Value::Null)
    );
    for d in report.items_of("donors") {
        assert_eq!(
            d.at("classification.audited_oss_inclusion"),
            Some(&Value::Null)
        );
    }
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, path: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(path).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                collect(root, &e.path(), out);
            } else {
                out.insert(
                    e.path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                    std::fs::read(e.path()).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    collect(root, root, &mut out);
    out
}

fn import_rows(r: &Repo, name: &str, rows: &str) -> String {
    let path = format!("research/intake/{name}.json");
    r.write(&path, &format!("{{\"donors\":{rows}}}\n"));
    import::import(
        r.path(),
        &path,
        &import::Options {
            kind: Some(UniverseKind::OssRepository),
            table: Some("donors".into()),
        },
        Some(&r.declaration()),
        false,
    )
    .unwrap();
    path
}

fn inactive(key: &str) -> Donor {
    let mut d = donor(key, key);
    d.claimed = DonorState::Registered;
    d.capabilities.clear();
    d.packages.clear();
    d.cutover = None;
    d
}

#[test]
fn report_is_deterministic_read_only_and_does_not_certify_oss_or_native_completion() {
    let r = extinct_baseline("audit-read-only");
    let before = tree(r.path());
    let first = audit::run(r.path()).unwrap().render();
    assert_eq!(audit::run(r.path()).unwrap().render(), first);
    assert_eq!(
        tree(r.path()),
        before,
        "audit may neither execute nor record proofs"
    );
    let value = json::parse(&first).unwrap();
    assert_eq!(value.str("format"), Some("ecdev-source-donor-audit-v1"));
    assert_eq!(integer(value.at("mixed_declared_cohort.total")), 1);
    assert_eq!(
        integer(value.at("mixed_declared_cohort.effective_extinct")),
        1
    );
    assert_eq!(
        value.at("snapshot.stable_before_after_assessment"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        value.at("snapshot.commit_content_match"),
        Some(&Value::Str("UNKNOWN_NOT_ATTESTED".into()))
    );
    assert!(value.text("binding").contains("NO_PROOFS_EXECUTED"));
    assert!(value.text("native_certification").contains("NOT_RUN"));
    unknown_oss(&value);
}

#[test]
fn imported_defaults_explicit_dataset_model_and_conflicts_retain_raw_claims() {
    for (name, fields, status, kinds) in [
        (
            "default",
            "\"donor_type\":\"TYPE_A\"",
            "UNRESOLVED_NO_EXPLICIT_CANONICAL_KIND",
            vec![],
        ),
        (
            "dataset",
            "\"type\":\"DATASET\"",
            "EXPLICIT_OTHER_KIND_CLAIM_UNVERIFIED",
            vec!["DATASET"],
        ),
        (
            "model",
            "\"source_kind\":\"MODEL\"",
            "UNRESOLVED_NO_EXPLICIT_CANONICAL_KIND",
            vec![],
        ),
        (
            "conflict",
            "\"type\":\"DATASET\",\"source_kind\":\"OSS_REPOSITORY\"",
            "CONFLICTING_EXPLICIT_KIND_CLAIMS",
            vec!["OSS_REPOSITORY", "DATASET"],
        ),
    ] {
        let r = extinct_baseline(name);
        let raw = format!("[{{\"id\":\"source-001\",\"repository\":\"https://github.com/georust/geo\",\"license\":\"MIT\",{fields}}}]");
        let path = import_rows(&r, name, &raw);
        let original = r.read(&path);
        let projection_path = import::universe_file(&path);
        let projection = r.read(&projection_path);
        let src = ecdev_governance::universe::Source::parse(&projection_path, &projection).unwrap();
        let report = report(&r);
        let row = donor_row(&report, "geo");
        assert_eq!(
            row.at("classification.status"),
            Some(&Value::Str(status.into())),
            "{name}"
        );
        let mut actual = row
            .at("classification.explicit_kind_claims")
            .unwrap()
            .items()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>();
        let mut expected = kinds;
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "{name}");
        let evidence = &row.at("classification.evidence").unwrap().items()[0];
        assert_eq!(evidence.str("record_id"), Some("source-001"));
        assert_eq!(evidence.str("imported_kind"), Some("OSS_REPOSITORY"));
        assert_eq!(
            evidence.get("imported_kind_is_reviewed_oss_decision"),
            Some(&Value::Bool(false))
        );
        assert_eq!(evidence.text("legacy_record"), src.records[0].legacy);
        assert_eq!(
            evidence.text("legacy_sha256"),
            content_digest(src.records[0].legacy.as_bytes())
        );
        assert_eq!(
            report.items_of("universe_record_evidence")[0].str("record_id"),
            Some("source-001")
        );
        assert_eq!(r.read(&path), original);
        assert_eq!(r.read(&projection_path), projection);
        unknown_oss(&report);
    }
}

#[test]
fn conflicting_records_of_one_origin_keep_both_ids_and_original_records() {
    let r = extinct_baseline("audit-conflicting-records");
    import_rows(&r, "code", "[{\"id\":\"code-claim\",\"repository\":\"https://github.com/georust/geo\",\"type\":\"OSS_REPOSITORY\"}]");
    import_rows(&r, "data", "[{\"id\":\"data-claim\",\"repository\":\"https://github.com/georust/geo\",\"type\":\"DATASET\"}]");
    let value = report(&r);
    let row = donor_row(&value, "geo");
    assert_eq!(
        row.at("classification.status"),
        Some(&Value::Str("CONFLICTING_EXPLICIT_KIND_CLAIMS".into()))
    );
    let evidence = row.at("classification.evidence").unwrap().items();
    assert_eq!(evidence.len(), 2);
    let ids = evidence
        .iter()
        .map(|v| v.text("record_id"))
        .collect::<Vec<_>>();
    assert!(ids.contains(&"code-claim".into()) && ids.contains(&"data-claim".into()));
    assert!(evidence.iter().all(|v| !v.text("legacy_record").is_empty()));
    assert_eq!(
        integer(value.at("mixed_declared_cohort.total")),
        1,
        "two claims do not create two declared donors"
    );
    unknown_oss(&value);
}

#[test]
fn declared_extinct_and_license_claim_never_override_stale_proofs_or_authorize_copying() {
    let r = extinct_baseline("audit-stale-proofs");
    r.edit(|d| d.donors[0].license = "MIT".into());
    import_rows(&r, "license", "[{\"id\":\"claim\",\"repository\":\"https://github.com/georust/geo\",\"license\":\"MIT\"}]");
    let current = report(&r);
    assert_eq!(
        donor_row(&current, "geo").str("effective_state"),
        Some("EXTINCT")
    );
    let original = r.read("substrate/geo/src/lib.rs");
    r.write(
        "substrate/geo/src/lib.rs",
        &(original + "\n// subject bytes changed after proof recording\n"),
    );
    let value = report(&r);
    let row = donor_row(&value, "geo");
    assert_eq!(row.str("declared_state"), Some("EXTINCT"));
    assert_eq!(row.get("effective_extinct"), Some(&Value::Bool(false)));
    assert_eq!(
        integer(value.at("mixed_declared_cohort.declared_extinct")),
        1
    );
    assert_eq!(
        integer(value.at("mixed_declared_cohort.effective_extinct")),
        0
    );
    assert_eq!(
        row.at("rights.declared_license"),
        Some(&Value::Str("MIT".into()))
    );
    assert_eq!(
        row.at("rights.registry_license_verified"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        row.at("rights.copying_authorization"),
        Some(&Value::Str("UNKNOWN_NOT_CERTIFIED_BY_THIS_REPORT".into()))
    );
    let cap = &row.items_of("capabilities")[0];
    assert!(cap
        .items_of("parity")
        .iter()
        .all(|p| p.str("verdict") == Some("STALE")));
    assert!(cap
        .items_of("regression")
        .iter()
        .all(|p| p.str("verdict") == Some("STALE")));
    for proof in cap.items_of("proof_bindings") {
        let records = proof.items_of("recorded_results");
        assert!(!records.is_empty());
        assert!(records
            .iter()
            .all(|x| x.get("recorded_passed") == Some(&Value::Bool(true))));
        assert!(records
            .iter()
            .all(|x| x.get("matches_current_bytes") == Some(&Value::Bool(false))));
        assert!(records
            .iter()
            .any(|x| x.text("subject_digest") != proof.text("current_subject_digest")));
    }
    unknown_oss(&value);
}

#[test]
fn caller_report_keeps_runtime_build_linked_test_import_resident_and_dependent_evidence() {
    let r = extinct_baseline("audit-callers");
    r.edit(|d| {
        d.donors[0].packages.push(Package {
            ecosystem: Ecosystem::Native,
            name: "geos".into(),
        });
        d.donors[0]
            .source_paths
            .push("research/checkouts/geo".into());
    });
    r.write("core/Cargo.toml", "[package]\nname=\"core-kernel\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[dependencies]\ngeo=\"0.33\"\n[build-dependencies]\ngeo=\"0.33\"\n[dev-dependencies]\ngeo=\"0.33\"\n");
    r.write("core/src/lib.rs", "use geo::Point;\n#[link(name = \"geos\")]\nextern \"C\" { fn GEOSversion() -> *const u8; }\npub fn id() -> u64 { 1 }\n");
    r.write(
        "research/checkouts/geo/original.rs",
        "pub fn original() {}\n",
    );
    let value = report(&r);
    let closure = donor_row(&value, "geo").get("caller_and_closure").unwrap();
    for field in [
        "runtime",
        "build",
        "linked",
        "test",
        "imports",
        "resident",
        "dependents",
    ] {
        assert!(
            !closure.get(field).unwrap().items().is_empty(),
            "missing {field}: {closure:?}"
        );
    }
    assert_eq!(closure.str("semantic_review"), Some("COMPLETE"));
    for field in ["runtime", "build", "linked", "test"] {
        for observation in closure.get(field).unwrap().items() {
            for key in ["file", "ecosystem", "name", "ident", "scope", "via"] {
                assert!(observation.get(key).is_some(), "{field}/{key}");
            }
        }
    }
    assert_eq!(
        donor_row(&value, "geo").get("effective_extinct"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn incomplete_or_absent_proofs_never_match_even_a_claim_with_equal_empty_digests() {
    for case in ["missing-subject", "missing-proof", "absent-test"] {
        let r = extinct_baseline(case);
        match case {
            "missing-subject" => r.edit(|d| d.donors[0].capabilities[0].replacement = None),
            "missing-proof" => r.remove("tests/tests/parity.rs"),
            "absent-test" => r.write("tests/tests/parity.rs", "pub fn helper() {}\n"),
            _ => unreachable!(),
        }
        let declaration = r.declaration();
        let capability = &declaration.donors[0].capabilities[0];
        let proof = &capability.proofs[0];
        let subject = capability.replacement.as_deref().unwrap_or("");
        let files = ecdev_governance::repository::files::Files::scan(r.path()).unwrap();
        let (proof_digest, subject_digest) =
            ecdev_governance::evidence::current_digests(&files, &declaration, proof, subject);
        let command = "synthetic nonbinding claim; never executed";
        ecdev_governance::evidence::Store::write(
            r.path(),
            &ecdev_governance::evidence::Record {
                locator: proof.locator.clone(),
                kind: proof.kind,
                passed: true,
                proof_digest,
                subject: subject.into(),
                subject_digest,
                command: command.into(),
            },
        )
        .unwrap();
        let value = report(&r);
        let row = donor_row(&value, "geo");
        let binding = row.items_of("capabilities")[0]
            .items_of("proof_bindings")
            .iter()
            .find(|b| b.str("locator") == Some(proof.locator.as_str()))
            .unwrap();
        let retained = binding
            .items_of("recorded_results")
            .iter()
            .find(|record| record.str("command") == Some(command))
            .unwrap();
        assert_eq!(retained.get("recorded_passed"), Some(&Value::Bool(true)));
        assert_eq!(
            retained.get("matches_current_bytes"),
            Some(&Value::Bool(false)),
            "{case}: {binding:?}"
        );
        assert_eq!(row.get("effective_extinct"), Some(&Value::Bool(false)));
    }
}

#[test]
fn vanished_history_and_legal_rejection_preserve_the_existing_gate_denominator() {
    let r = extinct_baseline("audit-history");
    r.edit(|d| {
        let mut rejected = inactive("rejected");
        rejected.exception = Some((ExceptionKind::Rejected, "reviewed, never adopted".into()));
        d.donors.push(rejected);
    });
    history::History::record(
        r.path(),
        &history::Batch {
            registered: ["geo".into(), "vanished".into(), "rejected".into()].into(),
            ..Default::default()
        },
    )
    .unwrap();
    let value = report(&r);
    assert_eq!(integer(value.at("mixed_declared_cohort.total")), 2);
    assert_eq!(integer(value.at("active_extinction_gate.numerator")), 1);
    assert_eq!(
        integer(value.at("active_extinction_gate.registered_active")),
        2
    );
    assert_eq!(integer(value.at("active_extinction_gate.denominator")), 2);
    assert_eq!(
        value.at("active_extinction_gate.pass"),
        Some(&Value::Bool(false))
    );
    let active = value
        .at("active_extinction_gate.active_keys")
        .unwrap()
        .items();
    assert!(
        active.contains(&Value::Str("geo".into()))
            && active.contains(&Value::Str("vanished".into()))
    );
    assert!(!active.contains(&Value::Str("rejected".into())));
    assert!(value
        .items_of("vanished_historical_donors")
        .contains(&Value::Str("vanished".into())));
    assert_eq!(
        donor_row(&value, "rejected").at("legal_exception.kind"),
        Some(&Value::Str(ExceptionKind::Rejected.wire().into()))
    );
}

#[test]
fn supplemental_untracked_state_and_census_inputs_change_the_bound_manifest() {
    let r = extinct_baseline("audit-index-supplement");
    // This Git index belongs only to the temporary fixture. No checkout's real index is edited.
    for args in [
        vec!["init", "-q", "--template="],
        vec![
            "add",
            "--",
            ".ecdev",
            "core",
            "substrate",
            "tests",
            "Cargo.toml",
            "README.md",
        ],
    ] {
        let result = Command::new("git")
            .current_dir(r.path())
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let initial = report(&r);
    assert_eq!(
        initial.at("snapshot.path_selection_from_index"),
        Some(&Value::Bool(true))
    );
    let census = "research/census/geo/summary.json";
    assert_eq!(input(&initial, census).str("state"), Some("PRESENT"));
    assert_eq!(
        input(&initial, census).text("sha256"),
        content_digest(r.read(census).as_bytes())
    );
    facts::Knowledge::add(
        r.path(),
        &[facts::Fact::new(
            FactKind::Decision,
            "source",
            "untracked",
            "UNKNOWN",
            "fixture receipt",
            1,
        )],
    )
    .unwrap();
    r.write(
        "research/commerce/dependencies.json",
        "{\"packages\":[{\"role\":\"RESEARCH_TOOL_DEPENDENCY\",\"source_kind\":\"MODEL\"}]}\n",
    );
    r.write(census, "{\"semantic_review\":\"PENDING\"}\n");
    let changed = report(&r);
    assert_ne!(
        initial.at("snapshot.input_manifest_sha256"),
        changed.at("snapshot.input_manifest_sha256")
    );
    assert_eq!(
        donor_row(&changed, "geo").at("caller_and_closure.semantic_review"),
        Some(&Value::Str("PENDING".into()))
    );
    assert_eq!(
        donor_row(&changed, "geo").get("effective_extinct"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        input(&changed, census).text("sha256"),
        content_digest(r.read(census).as_bytes())
    );
    assert!(changed
        .at("snapshot.inputs")
        .unwrap()
        .items()
        .iter()
        .any(|p| p.text("path").starts_with(".ecdev/knowledge/")
            && p.str("state") == Some("PRESENT")));
    assert_eq!(
        changed
            .at("participation_descriptors.records")
            .unwrap()
            .items()[0]
            .str("source_kind"),
        Some("MODEL")
    );
    unknown_oss(&changed);
}

#[test]
fn corrupt_control_state_remains_visible_and_cli_rejects_mutating_options() {
    let r = extinct_baseline("audit-corrupt");
    for path in [
        ".ecdev/history/bad.ecg",
        ".ecdev/evidence/bad.ecg",
        ".ecdev/knowledge/bad.ecg",
        ".ecdev/declared/universe/bad.jsonl",
    ] {
        r.write(path, "corrupt captured input\n");
    }
    let before = tree(r.path());
    let value = report(&r);
    for field in [
        "history_unreadable",
        "evidence_unreadable",
        "knowledge_unreadable",
        "universe_unreadable",
    ] {
        assert!(
            !value.items_of(field).is_empty(),
            "corruption disappeared from {field}"
        );
    }
    assert_eq!(
        input(&value, ".ecdev/history/bad.ecg").str("state"),
        Some("PRESENT")
    );
    for args in [
        vec!["donor", "audit", "--record"],
        vec!["donor", "audit", "--write"],
        vec!["donor", "audit", "--force"],
        vec!["donor", "audit", "--out", "report.json"],
        vec!["donor", "audit", "extra"],
    ] {
        let (code, out) = r.cli(&args);
        assert_eq!(code, 2, "{args:?}: {out}");
        assert!(out.contains("accepts only --root and --json"));
    }
    let (code, out) = r.cli(&["donor", "audit", "--json"]);
    assert_eq!(code, 0, "{out}");
    assert!(!json::parse(&out)
        .unwrap()
        .items_of("history_unreadable")
        .is_empty());
    assert_eq!(tree(r.path()), before);
}
