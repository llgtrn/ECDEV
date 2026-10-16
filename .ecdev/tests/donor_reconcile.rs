//! Source reviews classify retained evidence; they never supply lifecycle proofs.
mod common;

use common::Repo;
use ecdev_governance::digest::content_digest;
use ecdev_governance::donors::{audit, reconcile};
use ecdev_governance::formats::{json, json::Json, Value};
use ecdev_governance::schema::UniverseKind;
use ecdev_governance::universe::import;
use std::collections::BTreeMap;
use std::path::Path;

const LEDGER: &str = "research/commerce/governance/source-review-decisions-v1.json";
const PRICE: &str = "scrapinghub--price-parser";
const CENSUS: &str = "research/commerce/donors/census/scrapinghub--price-parser";

fn checkout() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn copy(root: &Path, destination: &Path) {
    if root.is_dir() {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            assert!(!entry.file_type().unwrap().is_symlink());
            copy(&entry.path(), &destination.join(entry.file_name()));
        }
    } else {
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(root, destination).unwrap();
    }
}

fn fixture(name: &str) -> Repo {
    let r = Repo::new(name);
    // These are retained records and data only. No other source checkout or
    // product implementation is copied, and no fixture proof is executed.
    for path in [
        ".ecdev/declared",
        ".ecdev/history",
        ".ecdev/evidence",
        ".ecdev/knowledge",
        CENSUS,
        "research/commerce/donors/registry.json",
        "research/commerce/donor-lifecycle-review.json",
        "research/commerce/price-parser-capability-review.json",
        "research/commerce/price-parser-exclusions.json",
        "research/commerce/price-parser-extinction.json",
        LEDGER,
    ] {
        copy(&checkout().join(path), &r.root.join(path));
    }
    r
}

fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, path: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), out);
            } else {
                out.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .into(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

fn report(r: &Repo) -> Value {
    json::parse(&reconcile::run(r.path()).unwrap().render()).unwrap()
}

fn row<'a>(report: &'a Value, alias: &str) -> &'a Value {
    report
        .items_of("per_donor")
        .iter()
        .find(|row| row.str("source_alias") == Some(alias))
        .unwrap()
}

fn rendered(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(v) => Json::from(*v),
        Value::Int(v) => Json::from(*v),
        Value::Str(v) => Json::from(v),
        Value::Float(_) => panic!("review fixture contains an unexpected float"),
        Value::Array(v) => Json::from(v.iter().map(rendered).collect::<Vec<_>>()),
        Value::Table(v) => Json::Object(v.iter().map(|(k, v)| (k.clone(), rendered(v))).collect()),
    }
}

fn edit_json(r: &Repo, path: &str, edit: impl FnOnce(&mut Value)) {
    let mut value = json::parse(&r.read(path)).unwrap();
    edit(&mut value);
    r.write(path, &rendered(&value).render());
}

fn field_mut<'a>(value: &'a mut Value, key: &str) -> &'a mut Value {
    match value {
        Value::Table(table) => table.get_mut(key).unwrap(),
        _ => panic!("expected an object containing {key}"),
    }
}

fn replace(r: &Repo, path: &str, old: &str, new: &str) {
    let original = r.read(path);
    assert!(
        original.contains(old),
        "fixture mutation must hit {path}: {old}"
    );
    assert_ne!(old, new);
    r.write(path, &original.replace(old, new));
}

fn rebind_receipt(r: &Repo, path: &str, edit: impl FnOnce(&mut Value)) {
    let old = content_digest(r.read(path).as_bytes());
    edit_json(r, path, edit);
    let new = content_digest(r.read(path).as_bytes());
    // Rebinding the outer evidence digest must not bypass the semantic
    // identity, pin, inventory or licence checks within the reviewed receipt.
    let ledger = r.read(LEDGER);
    if ledger.contains(&old) {
        replace(r, LEDGER, &old, &new);
    } else {
        replace(r, LEDGER, &old[7..], &new[7..]);
    }
}

fn rebind_text(r: &Repo, path: &str, changed: &str) {
    let old = content_digest(r.read(path).as_bytes());
    assert_ne!(r.read(path), changed);
    r.write(path, changed);
    replace(r, LEDGER, &old, &content_digest(changed.as_bytes()));
}

fn refused_without_writes(r: &Repo, name: &str) {
    let before = tree(r.path());
    assert!(
        reconcile::run(r.path()).is_err(),
        "{name} must refuse the entire report"
    );
    assert_eq!(tree(r.path()), before, "refusal must not rewrite {name}");
}

#[test]
fn bound_source_review_includes_one_source_and_preserves_all_unresolved_rows() {
    let r = fixture("reconcile-source-review");
    let before = tree(r.path());
    let first = reconcile::run(r.path()).unwrap().render();
    assert_eq!(first, reconcile::run(r.path()).unwrap().render());
    let value = json::parse(&first).unwrap();
    assert_eq!(
        value.str("format"),
        Some("ecdev-source-donor-reconciliation-v1")
    );
    assert_eq!(value.items_of("per_donor").len(), 68);
    assert_eq!(
        value.at("reviewed_source_oss.included"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        value.at("reviewed_source_oss.unresolved"),
        Some(&Value::Int(67))
    );
    assert_eq!(value.at("reviewed_source_oss.total"), Some(&Value::Null));
    assert_eq!(
        value.at("reviewed_source_oss.verified_extinct"),
        Some(&Value::Null)
    );
    assert_eq!(
        row(&value, PRICE).at("review.status"),
        Some(&Value::Str("SOURCE_REVIEWED_OSS_INCLUSION".into()))
    );
    assert_eq!(
        value
            .items_of("per_donor")
            .iter()
            .filter(|row| row.at("review.status") == Some(&Value::Str("UNRESOLVED".into())))
            .count(),
        67
    );
    assert_eq!(
        tree(r.path()),
        before,
        "review must not record evidence or modify captured bytes"
    );
}

#[test]
fn reviewed_inclusion_cannot_change_the_base_audit_or_active_extinction_gate() {
    let r = fixture("reconcile-gates-unchanged");
    let independent = json::parse(&audit::run(r.path()).unwrap().render()).unwrap();
    let reviewed = report(&r);
    assert_eq!(reviewed.get("source_audit"), Some(&independent));
    assert_eq!(
        reviewed.get("lifecycle_gates_modified"),
        Some(&Value::Bool(false))
    );
    assert_eq!(reviewed.get("new_assimilation"), Some(&Value::Int(0)));
    assert_eq!(reviewed.get("new_extinction"), Some(&Value::Int(0)));
    assert_eq!(reviewed.str("canonical_registry_binding"), Some("PENDING"));
    assert_eq!(
        row(&reviewed, PRICE).at("review.canonical_registry_binding"),
        Some(&Value::Str("PENDING".into()))
    );
    assert_eq!(
        row(&reviewed, PRICE).at("review.historical_license_receipt_status"),
        Some(&Value::Str("UNVERIFIED".into()))
    );
    assert!(row(&reviewed, PRICE)
        .at("review.current_copying_authorization")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("UNKNOWN"));
    for donor in independent.items_of("donors") {
        let reconciled = row(&reviewed, donor.str("source_alias").unwrap());
        assert_eq!(reconciled.get("source_id"), donor.get("source_id"));
        assert_eq!(
            reconciled.get("native_effective_state"),
            donor.get("effective_state")
        );
        assert_eq!(
            reconciled.get("active_gate_member"),
            donor.get("gate_member")
        );
        if reconciled.at("review.status") == Some(&Value::Str("UNRESOLVED".into())) {
            assert!(!reconciled
                .at("review.reason_codes")
                .unwrap()
                .items()
                .is_empty());
        }
    }
    r.remove(LEDGER);
    let unresolved = report(&r);
    for key in ["mixed_declared_cohort", "active_extinction_gate"] {
        assert_eq!(
            unresolved.at(&format!("source_audit.{key}")),
            independent.get(key)
        );
    }
    assert_eq!(
        unresolved.at("reviewed_source_oss.included"),
        Some(&Value::Int(0))
    );
}

#[test]
fn absent_decision_ledger_keeps_every_source_unresolved() {
    let r = fixture("reconcile-no-ledger");
    r.remove(LEDGER);
    let before = tree(r.path());
    let value = report(&r);
    assert_eq!(value.items_of("per_donor").len(), 68);
    assert_eq!(
        value.at("reviewed_source_oss.included"),
        Some(&Value::Int(0))
    );
    assert_eq!(
        value.at("reviewed_source_oss.unresolved"),
        Some(&Value::Int(68))
    );
    assert_eq!(value.at("reviewed_source_oss.total"), Some(&Value::Null));
    assert!(value
        .items_of("per_donor")
        .iter()
        .all(|row| row.at("review.status") == Some(&Value::Str("UNRESOLVED".into()))));
    assert_eq!(tree(r.path()), before);
}

#[test]
fn missing_and_changed_bound_receipts_refuse_instead_of_disappearing() {
    for file in [
        "identity.json",
        "license.json",
        "files.jsonl",
        "tree.json",
        "summary.json",
    ] {
        let r = fixture(&format!("reconcile-missing-{file}"));
        r.remove(&format!("{CENSUS}/{file}"));
        refused_without_writes(&r, file);
        let r = fixture(&format!("reconcile-changed-{file}"));
        let path = format!("{CENSUS}/{file}");
        r.write(&path, &(r.read(&path) + " \n"));
        refused_without_writes(&r, file);
    }
}

#[test]
fn rebound_identity_receipts_cannot_substitute_a_pin_or_lookalike_origin() {
    for (field, value) in [
        ("commit_sha", "0000000000000000000000000000000000000000"),
        (
            "repository_url",
            "https://github.com/another-owner/price-parser",
        ),
        (
            "resolved_remote_url",
            "https://github.com/another-owner/price-parser.git",
        ),
        ("donor_id", "another-owner--price-parser"),
    ] {
        let r = fixture(&format!("reconcile-identity-{field}"));
        rebind_receipt(&r, &format!("{CENSUS}/identity.json"), |identity| {
            *field_mut(identity, field) = Value::Str(value.into());
        });
        refused_without_writes(&r, field);
    }
}

#[test]
fn declaration_and_selected_legacy_record_must_still_match_the_reviewed_identity() {
    for (field, value) in [
        ("origin", "https://github.com/another-owner/price-parser"),
        ("license", "UNKNOWN"),
    ] {
        let r = fixture(&format!("reconcile-declaration-{field}"));
        r.edit(|declaration| {
            let donor = declaration
                .donors
                .iter_mut()
                .find(|d| d.key == PRICE)
                .unwrap();
            if field == "origin" {
                donor.origin = value.into();
            } else {
                donor.license = value.into();
            }
        });
        refused_without_writes(&r, field);
    }
    let r = fixture("reconcile-raw-record-changed");
    let path = ".ecdev/declared/universe/research.commerce.donors.registry.json.jsonl";
    let mut changed = 0;
    let lines = r
        .read(path)
        .lines()
        .map(|line| {
            let mut record = json::parse(line).unwrap();
            if record.str("id") == Some("#21") {
                let raw = record.text("legacy");
                assert!(raw.contains("scrapinghub--price-parser"));
                *field_mut(&mut record, "legacy") = Value::Str(raw + " ");
                changed += 1;
            }
            rendered(&record).render_line()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        changed, 1,
        "the selected legacy record must be mutated exactly once"
    );
    r.write(path, &(lines.join("\n") + "\n"));
    refused_without_writes(
        &r,
        "raw legacy record bytes changed while facts still look alike",
    );
}

#[test]
fn retained_licence_tampering_cannot_be_laundered_by_recomputing_its_own_hash() {
    let r = fixture("reconcile-license-self-hash");
    rebind_receipt(&r, &format!("{CENSUS}/license.json"), |license| {
        let files = field_mut(license, "files");
        let Value::Array(files) = files else {
            panic!("licence files must be an array")
        };
        let file = &mut files[0];
        let changed = format!(
            "{}\nChanged licence terms for this negative fixture.\n",
            file.text("text")
        );
        *field_mut(file, "text") = Value::Str(changed.clone());
        *field_mut(file, "sha256") = Value::Str(content_digest(changed.as_bytes())[7..].into());
    });
    refused_without_writes(&r, "licence text with a recomputed self-reported digest");
}

#[test]
fn rebound_inventory_cannot_omit_a_source_file_or_invent_a_tree_entry() {
    let r = fixture("reconcile-tree-extra");
    rebind_receipt(&r, &format!("{CENSUS}/tree.json"), |tree| {
        let Value::Array(files) = field_mut(tree, "files") else {
            panic!("tree files must be an array")
        };
        files.push(Value::Str("unreviewed-model-weights.bin".into()));
    });
    refused_without_writes(&r, "unreviewed tree entry");
    let r = fixture("reconcile-source-file-missing");
    let path = format!("{CENSUS}/files.jsonl");
    let original = r.read(&path);
    let old = content_digest(original.as_bytes());
    let changed = original
        .lines()
        .filter(|line| json::parse(line).unwrap().str("path") != Some("price_parser/parser.py"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(original, changed);
    r.write(&path, &changed);
    let new = content_digest(changed.as_bytes());
    let ledger = r.read(LEDGER);
    if ledger.contains(&old) {
        replace(&r, LEDGER, &old, &new);
    } else {
        replace(&r, LEDGER, &old[7..], &new[7..]);
    }
    refused_without_writes(&r, "missing first-party source inventory row");

    let r = fixture("reconcile-inventory-duplicate");
    let original = r.read(&path);
    let old = content_digest(original.as_bytes());
    let changed = format!("{}{}\n", original, original.lines().next().unwrap());
    r.write(&path, &changed);
    let new = content_digest(changed.as_bytes());
    replace(&r, LEDGER, &old, &new);
    refused_without_writes(
        &r,
        "duplicate inventory path despite a rebound receipt digest",
    );
}

#[test]
fn additional_conflicting_records_cannot_hide_behind_the_selected_record() {
    for (field, changed) in [
        ("type", "DATASET"),
        ("commit_sha", "0000000000000000000000000000000000000000"),
        ("license", "Proprietary"),
        (
            "repository",
            "https://github.com/another-owner/price-parser",
        ),
    ] {
        let r = fixture(&format!("reconcile-conflicting-record-{field}"));
        let mut raw = json::parse(&format!(
            "{{\"id\":\"additional-claim\",\"donor_id\":{},\"repository\":\"https://github.com/scrapinghub/price-parser\",\"commit_sha\":\"64e213a46a40473ba4f8aa3b249917fdc64d8a16\",\"license\":\"BSD-3-Clause\",\"type\":\"OSS_REPOSITORY\"}}",
            Json::from(PRICE).render_line(),
        )).unwrap();
        *field_mut(&mut raw, field) = Value::Str(changed.into());
        let path = "research/commerce/additional-source-claim.json";
        r.write(
            path,
            &Json::obj().with("donors", vec![rendered(&raw)]).render(),
        );
        import::import(
            r.path(),
            path,
            &import::Options {
                kind: Some(UniverseKind::OssRepository),
                table: Some("donors".into()),
            },
            Some(&r.declaration()),
            false,
        )
        .unwrap();
        refused_without_writes(&r, field);
    }
}

#[test]
fn rebound_inventory_metadata_and_census_counts_must_remain_well_formed() {
    for (field, changed) in [
        ("sha256", Value::Str("not-a-digest".into())),
        ("blob_hash", Value::Str("not-a-git-blob".into())),
        ("git_mode", Value::Str("120000".into())),
        ("bytes", Value::Int(-1)),
    ] {
        let r = fixture(&format!("reconcile-inventory-shape-{field}"));
        let path = format!("{CENSUS}/files.jsonl");
        let lines = r
            .read(&path)
            .lines()
            .map(|line| {
                let mut file = json::parse(line).unwrap();
                if file.str("path") == Some("price_parser/parser.py") {
                    *field_mut(&mut file, field) = changed.clone();
                }
                rendered(&file).render_line()
            })
            .collect::<Vec<_>>();
        rebind_text(&r, &path, &(lines.join("\n") + "\n"));
        refused_without_writes(&r, field);
    }
    for (field, changed) in [
        ("total_files", 15),
        ("first_party_source_files", 2),
        ("source_parsed", 2),
        ("parse_unknown", 1),
        ("unknown_files", 1),
    ] {
        let r = fixture(&format!("reconcile-summary-shape-{field}"));
        rebind_receipt(&r, &format!("{CENSUS}/summary.json"), |summary| {
            *field_mut(summary, field) = Value::Int(changed);
        });
        refused_without_writes(&r, field);
    }
}

#[test]
fn agreeing_inventory_and_tree_cannot_admit_parent_traversal() {
    let r = fixture("reconcile-inventory-traversal");
    let old = "price_parser/parser.py";
    let new = "../price_parser/parser.py";
    let inventory_path = format!("{CENSUS}/files.jsonl");
    let original = r.read(&inventory_path);
    assert!(original.contains(old));
    rebind_text(&r, &inventory_path, &original.replace(old, new));
    rebind_receipt(&r, &format!("{CENSUS}/tree.json"), |tree| {
        let Value::Array(files) = field_mut(tree, "files") else {
            panic!("tree files must be an array")
        };
        let path = files
            .iter_mut()
            .find(|path| path.as_str() == Some(old))
            .unwrap();
        *path = Value::Str(new.into());
    });
    refused_without_writes(&r, "matching unsafe paths across the tree and inventory");
}

#[test]
fn strict_review_json_keeps_legacy_duplicate_key_semantics_separate() {
    for (raw, key) in [
        (r#"{"key":1,"key":2}"#, "key"),
        (r#"{"outer":{"key":1,"\u006bey":2}}"#, "outer.key"),
    ] {
        assert_eq!(json::parse(raw).unwrap().at(key), Some(&Value::Int(2)));
        assert!(json::parse_unique(raw).unwrap_err().contains("duplicate"));
    }
    assert_eq!(
        json::parse_unique(r#"{"key":2}"#).unwrap().get("key"),
        Some(&Value::Int(2))
    );
}

#[test]
fn malformed_duplicate_and_unknown_ledger_fields_never_grant_partial_credit() {
    let r = fixture("reconcile-ledger-shape");
    let original = r.read(LEDGER);
    let parsed = json::parse(&original).unwrap();
    let format = parsed.str("format").unwrap();
    let duplicate_format = format!(
        "{{\"format\":{},{}",
        Json::from(format).render_line(),
        original.trim_start().strip_prefix('{').unwrap()
    );
    for (name, changed) in [
        ("truncated", original[..original.len() / 2].into()),
        ("duplicate root field", duplicate_format),
        (
            "unknown version",
            original.replace(format, "ecdev-source-review-decisions-v999"),
        ),
        (
            "unknown root field",
            format!(
                "{{\"unreviewed_override\":true,{}",
                original.trim_start().strip_prefix('{').unwrap()
            ),
        ),
    ] {
        assert_ne!(changed, original, "{name}");
        r.write(LEDGER, &changed);
        refused_without_writes(&r, name);
    }
}

#[test]
fn duplicate_reviews_evidence_paths_and_unknown_nested_fields_refuse() {
    for mutation in [
        "review",
        "evidence",
        "review field",
        "record field",
        "license field",
    ] {
        let r = fixture(&format!("reconcile-ledger-{mutation}"));
        edit_json(&r, LEDGER, |ledger| {
            let Value::Array(reviews) = field_mut(ledger, "reviews") else {
                panic!("reviews must be an array")
            };
            if mutation == "review" {
                reviews.push(reviews[0].clone());
                return;
            }
            let review = &mut reviews[0];
            if mutation == "evidence" {
                let Value::Array(evidence) = field_mut(review, "evidence") else {
                    panic!("evidence must be an array")
                };
                evidence.push(evidence[0].clone());
                return;
            }
            let object = match mutation {
                "record field" => field_mut(review, "record"),
                "license field" => field_mut(review, "retained_root_license"),
                _ => review,
            };
            let Value::Table(table) = object else {
                panic!("review field must belong to an object")
            };
            table.insert("unreviewed_override".into(), Value::Bool(true));
        });
        refused_without_writes(&r, mutation);
    }
    let r = fixture("reconcile-decoded-duplicate");
    replace(
        &r,
        LEDGER,
        "\"source_alias\":",
        &format!(
            "\"\\u0073ource_alias\": {}, \"source_alias\":",
            Json::from(PRICE).render_line()
        ),
    );
    refused_without_writes(&r, "duplicate decoded nested JSON key");
}

#[test]
fn caller_rejects_mutation_and_extra_inputs_without_writing() {
    let r = fixture("reconcile-caller");
    let before = tree(r.path());
    for args in [
        vec!["donor", "reconcile", "--record"],
        vec!["donor", "reconcile", "--write"],
        vec!["donor", "reconcile", "--force"],
        vec!["donor", "reconcile", "--out", "report.json"],
        vec!["donor", "reconcile", "--from", LEDGER],
        vec!["donor", "reconcile", "extra"],
    ] {
        let (code, output) = r.cli(&args);
        assert_eq!(code, 2, "{args:?}: {output}");
        assert!(
            output.contains("accepts only --root and --json"),
            "{output}"
        );
    }
    let (code, output) = r.cli(&["donor", "reconcile", "--json"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(
        json::parse(&output).unwrap().items_of("per_donor").len(),
        68
    );
    assert_eq!(tree(r.path()), before);
}
