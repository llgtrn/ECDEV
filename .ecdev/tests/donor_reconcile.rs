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
const EXTRUCT: &str = "scrapinghub--extruct";
const EXTRUCT_CENSUS: &str = "research/commerce/donors/census/scrapinghub--extruct";
const REGISTRY_RECORDS: &str =
    ".ecdev/declared/universe/research.commerce.donors.registry.json.jsonl";
const EXTRUCT_REVISION: &str = "dc3bf7d2209ecf421222afc90d3d2dd8c6fd23cf";
const EXTRUCT_LEGACY_SHA: &str =
    "sha256:8d0f6f77db58eff56160eacf356c010e104a95669608d87b03db9db2a49ab8df";
const CRAWLEE: &str = "apify--crawlee";
const CRAWLEE_CENSUS: &str = "research/commerce/donors/census/apify--crawlee";

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
    let r = copied_fixture(name);
    select_reviews(&r, &[PRICE]);
    r
}

fn copied_fixture(name: &str) -> Repo {
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

fn select_reviews(r: &Repo, aliases: &[&str]) {
    edit_json(r, LEDGER, |ledger| {
        let Value::Array(reviews) = field_mut(ledger, "reviews") else {
            panic!("reviews must be an array")
        };
        reviews.retain(|review| aliases.contains(&review.str("source_alias").unwrap()));
        assert_eq!(reviews.len(), aliases.len());
    });
    assert_selected_reviews_match_checkout(r, aliases);
}

fn assert_selected_reviews_match_checkout(r: &Repo, aliases: &[&str]) {
    let actual =
        json::parse_unique(&std::fs::read_to_string(checkout().join(LEDGER)).unwrap()).unwrap();
    let selected = json::parse_unique(&r.read(LEDGER)).unwrap();
    for key in ["format", "scope"] {
        assert_eq!(selected.get(key), actual.get(key));
    }
    assert_eq!(selected.items_of("reviews").len(), aliases.len());
    for alias in aliases {
        let select = |ledger: &Value| {
            ledger
                .items_of("reviews")
                .iter()
                .filter(|review| review.str("source_alias") == Some(*alias))
                .cloned()
                .collect::<Vec<_>>()
        };
        let actual_review = select(&actual);
        assert_eq!(actual_review.len(), 1);
        assert_eq!(select(&selected), actual_review);
    }
}

fn two_source_fixture(name: &str) -> Repo {
    let r = copied_fixture(name);
    for path in [
        EXTRUCT_CENSUS,
        "research/commerce/extruct-semantic-matrix.json",
        "research/commerce/extruct-extinction.json",
    ] {
        copy(&checkout().join(path), &r.root.join(path));
    }
    select_reviews(&r, &[PRICE, EXTRUCT]);
    r
}

fn three_source_fixture(name: &str) -> Repo {
    let r = two_source_fixture(name);
    for path in [
        LEDGER,
        CRAWLEE_CENSUS,
        "research/commerce/crawlee-capability-review.json",
        "research/commerce/frontier-source-review.json",
    ] {
        copy(&checkout().join(path), &r.root.join(path));
    }
    assert_selected_reviews_match_checkout(&r, &[PRICE, EXTRUCT, CRAWLEE]);
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

fn refused_with(r: &Repo, name: &str, expected: &str) {
    let before = tree(r.path());
    let error = reconcile::run(r.path()).unwrap_err();
    assert!(
        error.contains(expected),
        "{name}: expected {expected:?}, received {error:?}"
    );
    assert_eq!(tree(r.path()), before, "refusal must not rewrite {name}");
}

fn extruct_review_mut(ledger: &mut Value) -> &mut Value {
    let Value::Array(reviews) = field_mut(ledger, "reviews") else {
        panic!("reviews must be an array")
    };
    reviews
        .iter_mut()
        .find(|review| review.str("source_alias") == Some(EXTRUCT))
        .unwrap()
}

fn edit_extruct_legacy(r: &Repo, rebind: bool, edit: impl FnOnce(&mut Value)) {
    let mut source =
        ecdev_governance::universe::Source::parse(REGISTRY_RECORDS, &r.read(REGISTRY_RECORDS))
            .unwrap();
    assert_eq!(
        source
            .records
            .iter()
            .filter(|record| record.id == "#20")
            .count(),
        1,
    );
    let selected = source
        .records
        .iter_mut()
        .find(|record| record.id == "#20")
        .unwrap();
    let original = selected.legacy.clone();
    assert_eq!(content_digest(original.as_bytes()), EXTRUCT_LEGACY_SHA);
    let mut legacy = json::parse_unique(&original).unwrap();
    edit(&mut legacy);
    let changed = rendered(&legacy).render();
    assert_ne!(changed, original);
    selected.legacy = changed;
    // Keep the hostile fixture's original registry and its lossless projection
    // consistent, so these cases reach the review's own record-binding checks.
    // Editing only JSONL would stop earlier at the Universe custody guard.
    r.write(&source.source, &source.rebuild());
    let imported = import::import(
        r.path(),
        &source.source,
        &import::Options {
            kind: Some(UniverseKind::OssRepository),
            table: Some("donors".into()),
        },
        Some(&r.declaration()),
        false,
    )
    .unwrap();
    let imported_legacy = &imported
        .source
        .records
        .iter()
        .find(|record| record.id == "#20")
        .unwrap()
        .legacy;
    assert_eq!(json::parse_unique(imported_legacy).unwrap(), legacy);
    let changed_sha = content_digest(imported_legacy.as_bytes());
    ecdev_governance::universe::Source::parse(REGISTRY_RECORDS, &r.read(REGISTRY_RECORDS))
        .expect("mutated fixture must retain a readable lossless projection");
    if rebind {
        edit_json(r, LEDGER, |ledger| {
            let selector = field_mut(extruct_review_mut(ledger), "record");
            *field_mut(selector, "legacy_sha256") = Value::Str(changed_sha);
        });
    }
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

#[test]
fn two_actual_reviews_preserve_exact_extruct_identity_terms_and_captured_bytes() {
    let r = two_source_fixture("reconcile-two-actual-sources");
    let before = tree(r.path());
    for (path, bytes) in &before {
        if path == LEDGER {
            assert_selected_reviews_match_checkout(&r, &[PRICE, EXTRUCT]);
            continue;
        }
        assert_eq!(
            *bytes,
            std::fs::read(checkout().join(path)).unwrap(),
            "the two-source fixture must retain actual bytes: {path}"
        );
    }
    let first = reconcile::run(r.path()).unwrap().render();
    assert_eq!(first, reconcile::run(r.path()).unwrap().render());
    let value = json::parse_unique(&first).unwrap();
    assert_eq!(value.items_of("per_donor").len(), 68);
    assert_eq!(
        value.at("reviewed_source_oss.included"),
        Some(&Value::Int(2))
    );
    assert_eq!(
        value.at("reviewed_source_oss.unresolved"),
        Some(&Value::Int(66))
    );
    for field in ["total", "verified_extinct"] {
        assert_eq!(
            value.at(&format!("reviewed_source_oss.{field}")),
            Some(&Value::Null)
        );
    }
    assert_eq!(
        value
            .items_of("per_donor")
            .iter()
            .filter(|donor| donor.at("review.status") == Some(&Value::Str("UNRESOLVED".into())))
            .count(),
        66
    );
    let extruct = row(&value, EXTRUCT);
    for (key, expected) in [
        ("source_id", "ec1:00f787db006a0e71f5076f9f9144b79d"),
        ("origin", "https://github.com/scrapinghub/extruct"),
        ("review.status", "SOURCE_REVIEWED_OSS_INCLUSION"),
        ("review.source_kind", "OSS_REPOSITORY"),
        ("review.upstream_revision", EXTRUCT_REVISION),
        ("review.global_origin", "github.com/scrapinghub/extruct"),
        ("review.original_record_id", "#20"),
        (
            "review.original_provenance",
            "research/commerce/donors/registry.json##20",
        ),
        ("review.declared_license", "BSD-3-Clause"),
        ("review.historical_license_receipt_status", "UNVERIFIED"),
        ("review.retained_root_license.path", "LICENSE"),
        (
            "review.retained_root_license.sha256",
            "sha256:3867c3ec55d2088f90b7e5559b5177ddacce592e6be255291294d6a238b00c1b",
        ),
        (
            "review.retained_root_license.blob",
            "737fbfaae515cd1f90c361857a7bca399aface65",
        ),
        ("review.canonical_registry_binding", "PENDING"),
    ] {
        assert_eq!(
            extruct.at(key).and_then(Value::as_str),
            Some(expected),
            "{key}"
        );
    }
    for (key, expected) in [
        ("review.inventory_files", 176),
        ("review.first_party_source_files", 14),
        ("review.retained_root_license.bytes", 1509),
    ] {
        assert_eq!(extruct.at(key), Some(&Value::Int(expected)), "{key}");
    }
    assert_eq!(extruct.at("review.oss_inclusion"), Some(&Value::Bool(true)));
    for alias in [PRICE, EXTRUCT] {
        assert!(row(&value, alias)
            .at("review.current_copying_authorization")
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("UNKNOWN"));
    }
    let evidence = [
        (
            format!("{EXTRUCT_CENSUS}/identity.json"),
            "sha256:6765966953bee44d73395c0025c723cfadec75aabd19c21452d44e086a31d7d4",
        ),
        (
            format!("{EXTRUCT_CENSUS}/license.json"),
            "sha256:e868a63921ea4b6426baf3f20757a4ffdbd314db6fbad49365e702bd03bb194f",
        ),
        (
            format!("{EXTRUCT_CENSUS}/files.jsonl"),
            "sha256:aef99dc532619c10c12f6c638bb61f473e8e0d1869a52c5d14093989446992ee",
        ),
        (
            format!("{EXTRUCT_CENSUS}/tree.json"),
            "sha256:9b22c58830de2c8f18748afa094fdd60afb342d7099ca1909ea892fe29cbd54f",
        ),
        (
            format!("{EXTRUCT_CENSUS}/summary.json"),
            "sha256:1843979ae012dcd15d4fca33acec253790ba4747476fe2d91f6b7304925dd460",
        ),
        (
            "research/commerce/donor-lifecycle-review.json".into(),
            "sha256:33fe116ddb497b26336b65bc28ce390cebb67bb6530cce3b01b668f8dc57c33c",
        ),
    ];
    for (path, expected) in evidence {
        assert_eq!(content_digest(&before[&path]), expected, "{path}");
        let bound = value
            .items_of("supplemental_inputs")
            .iter()
            .find(|input| input.str("path") == Some(path.as_str()))
            .unwrap();
        assert_eq!(bound.str("sha256"), Some(expected));
    }
    let original = r
        .read(REGISTRY_RECORDS)
        .lines()
        .map(|line| json::parse_unique(line).unwrap())
        .find(|record| record.str("id") == Some("#20"))
        .unwrap();
    assert_eq!(
        content_digest(original.text("legacy").as_bytes()),
        EXTRUCT_LEGACY_SHA
    );
    let identity = json::parse_unique(&r.read(&format!("{EXTRUCT_CENSUS}/identity.json"))).unwrap();
    assert_eq!(identity.str("donor_type"), Some("UNCLASSIFIED"));
    let (code, output) = r.cli(&["donor", "reconcile", "--json"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(json::parse_unique(&output).unwrap(), value);
    assert_eq!(tree(r.path()), before);
}

#[test]
fn second_source_inclusion_cannot_change_native_lifecycle_rights_or_audit_decisions() {
    let r = two_source_fixture("reconcile-two-native-boundaries");
    let before = tree(r.path());
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
        independent.at("mixed_declared_cohort.total"),
        Some(&Value::Int(68))
    );
    assert_eq!(
        independent.at("mixed_declared_cohort.declared_extinct"),
        Some(&Value::Int(10))
    );
    assert_eq!(
        independent.at("active_extinction_gate.pass"),
        Some(&Value::Bool(false))
    );
    assert_eq!(independent.at("audited_oss.total"), Some(&Value::Null));
    assert_eq!(
        independent.at("audited_oss.verified_extinct"),
        Some(&Value::Null)
    );
    for donor in independent.items_of("donors") {
        let reconciled = row(&reviewed, donor.str("source_alias").unwrap());
        for (review_key, audit_key) in [
            ("source_id", "source_id"),
            ("native_effective_state", "effective_state"),
            ("active_gate_member", "gate_member"),
            ("native_stopped_by", "stopped_by"),
        ] {
            assert_eq!(reconciled.get(review_key), donor.get(audit_key));
        }
    }
    let extruct = independent
        .items_of("donors")
        .iter()
        .find(|donor| donor.str("source_alias") == Some(EXTRUCT))
        .unwrap();
    assert_eq!(
        extruct.at("classification.audited_oss_inclusion"),
        Some(&Value::Null)
    );
    assert!(extruct
        .at("rights.copying_authorization")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("UNKNOWN"));
    assert_eq!(tree(r.path()), before);

    // Selecting one or zero review decisions changes only the reconciliation view.
    // Missing product source in this data-only fixture must not be called fresh
    // extinction evidence or made to match the production numerator.
    select_reviews(&r, &[PRICE]);
    let price_only = report(&r);
    assert_eq!(
        price_only.at("reviewed_source_oss.included"),
        Some(&Value::Int(1))
    );
    assert_eq!(
        row(&price_only, EXTRUCT).at("review.status"),
        Some(&Value::Str("UNRESOLVED".into()))
    );
    r.remove(LEDGER);
    let no_reviews = report(&r);
    for projected in [&price_only, &no_reviews] {
        for key in [
            "mixed_declared_cohort",
            "active_extinction_gate",
            "classification_summary",
            "audited_oss",
            "donors",
            "universe_record_evidence",
            "universe_relations",
            "universe_sources",
            "unregistered_participation",
            "vanished_historical_donors",
            "findings",
        ] {
            assert_eq!(
                projected.at(&format!("source_audit.{key}")),
                independent.get(key),
                "{key}"
            );
        }
    }
    for (path, bytes) in before {
        if path != LEDGER {
            assert_eq!(std::fs::read(r.root.join(&path)).unwrap(), bytes, "{path}");
        }
    }
}

#[test]
fn missing_or_tampered_second_source_evidence_refuses_both_reviews_without_writes() {
    for path in [
        format!("{EXTRUCT_CENSUS}/identity.json"),
        format!("{EXTRUCT_CENSUS}/license.json"),
        format!("{EXTRUCT_CENSUS}/files.jsonl"),
        format!("{EXTRUCT_CENSUS}/tree.json"),
        format!("{EXTRUCT_CENSUS}/summary.json"),
        "research/commerce/donor-lifecycle-review.json".into(),
    ] {
        let r = two_source_fixture("reconcile-second-missing");
        r.remove(&path);
        refused_without_writes(&r, &path);
        let r = two_source_fixture("reconcile-second-tampered");
        r.write(&path, &(r.read(&path) + " \n"));
        refused_with(&r, &path, "source review sha256 mismatch");
    }
}

#[test]
fn rebound_second_source_receipts_still_require_original_terms_and_closed_census() {
    let r = two_source_fixture("reconcile-second-license-rebound");
    rebind_receipt(&r, &format!("{EXTRUCT_CENSUS}/license.json"), |license| {
        let Value::Array(files) = field_mut(license, "files") else {
            panic!("licence files must be an array")
        };
        let changed = format!("{}\nSubstituted terms.\n", files[0].text("text"));
        *field_mut(&mut files[0], "text") = Value::Str(changed.clone());
        *field_mut(&mut files[0], "sha256") =
            Value::Str(content_digest(changed.as_bytes())[7..].into());
    });
    refused_with(
        &r,
        "rebound license self-hash",
        "retained license text mismatch",
    );

    let r = two_source_fixture("reconcile-second-inventory-rebound");
    let path = format!("{EXTRUCT_CENSUS}/files.jsonl");
    let original = r.read(&path);
    let changed = original
        .lines()
        .filter(|line| json::parse_unique(line).unwrap().str("path") != Some("extruct/jsonld.py"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    rebind_text(&r, &path, &changed);
    refused_with(
        &r,
        "missing source behind rebound inventory",
        "review inventory/tree/summary coverage differs",
    );

    let r = two_source_fixture("reconcile-second-summary-rebound");
    rebind_receipt(&r, &format!("{EXTRUCT_CENSUS}/summary.json"), |summary| {
        *field_mut(summary, "source_parsed") = Value::Int(13);
    });
    refused_with(
        &r,
        "rebound parse claim",
        "review closed source census counts or parse assertions disagree",
    );

    let r = two_source_fixture("reconcile-second-submodule-rebound");
    rebind_receipt(&r, &format!("{EXTRUCT_CENSUS}/tree.json"), |tree| {
        *field_mut(tree, "submodules") =
            Value::Array(vec![Value::Str("unreviewed-component".into())]);
    });
    refused_with(
        &r,
        "unreviewed submodule",
        "submodule custody needs a separate reviewed profile",
    );

    for (field, changed, expected) in [
        (
            "commit_sha",
            "0000000000000000000000000000000000000000",
            "source review commit_sha mismatch",
        ),
        ("license", "MIT", "source review license mismatch"),
    ] {
        let r = two_source_fixture(&format!("reconcile-second-lifecycle-{field}"));
        rebind_receipt(
            &r,
            "research/commerce/donor-lifecycle-review.json",
            |lifecycle| {
                let Value::Array(donors) = field_mut(lifecycle, "donors") else {
                    panic!("historical donors must be an array")
                };
                let extruct = donors
                    .iter_mut()
                    .find(|d| d.str("donor_id") == Some(EXTRUCT))
                    .unwrap();
                *field_mut(extruct, field) = Value::Str(changed.into());
            },
        );
        refused_with(&r, field, expected);
    }
}

#[test]
fn original_second_source_record_bytes_identity_and_revision_cannot_be_rebound() {
    let r = two_source_fixture("reconcile-second-original-bytes");
    edit_extruct_legacy(&r, false, |_| {});
    refused_with(
        &r,
        "equivalent legacy JSON reencoded",
        "source review legacy_sha256 mismatch",
    );

    for (field, changed, expected) in [
        (
            "donor_id",
            "another-owner--extruct",
            "source review donor_id mismatch",
        ),
        (
            "commit_sha",
            "0000000000000000000000000000000000000000",
            "source review commit_sha mismatch",
        ),
        ("license", "MIT", "source review license mismatch"),
        (
            "repository_url",
            "https://github.com/another-owner/extruct",
            "conflicting explicit same-record origin: repository_url",
        ),
    ] {
        let r = two_source_fixture(&format!("reconcile-second-original-{field}"));
        edit_extruct_legacy(&r, true, |legacy| {
            *field_mut(legacy, field) = Value::Str(changed.into());
        });
        refused_with(&r, field, expected);
    }
    let r = two_source_fixture("reconcile-second-selector-missing");
    edit_json(&r, LEDGER, |ledger| {
        *field_mut(field_mut(extruct_review_mut(ledger), "record"), "record_id") =
            Value::Str("#missing".into());
    });
    refused_with(
        &r,
        "nonexistent original record",
        "review requires exactly one original source record",
    );
}

#[test]
fn second_source_conflicting_alias_or_origin_records_refuse_partial_credit() {
    for (field, changed, expected) in [
        ("type", "DATASET", "conflicting source kind field: type"),
        (
            "commit_sha",
            "0000000000000000000000000000000000000000",
            "conflicting same-origin record commit_sha",
        ),
        (
            "license",
            "Proprietary",
            "conflicting same-origin record license",
        ),
        (
            "donor_id",
            "another-alias--extruct",
            "conflicting same-origin record donor_id",
        ),
        (
            "repository",
            "https://github.com/another-owner/extruct",
            "same-alias source record has conflicting imported origin",
        ),
    ] {
        let r = two_source_fixture(&format!("reconcile-second-conflict-{field}"));
        let mut raw = json::parse_unique(
            &Json::obj()
                .with("id", "second-source-conflict")
                .with("donor_id", EXTRUCT)
                .with("repository", "https://github.com/scrapinghub/extruct")
                .with("commit_sha", EXTRUCT_REVISION)
                .with("license", "BSD-3-Clause")
                .with("type", "OSS_REPOSITORY")
                .render(),
        )
        .unwrap();
        *field_mut(&mut raw, field) = Value::Str(changed.into());
        let path = "research/commerce/additional-second-source-claim.json";
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
        refused_with(&r, field, expected);
    }
}

#[test]
fn three_actual_reviews_classify_crawlee_without_completing_its_framework_or_lifecycle() {
    let r = three_source_fixture("reconcile-three-actual-sources");
    let before = tree(r.path());
    for (path, bytes) in &before {
        assert_eq!(
            *bytes,
            std::fs::read(checkout().join(path)).unwrap(),
            "{path}"
        );
    }
    let independent = json::parse_unique(&audit::run(r.path()).unwrap().render()).unwrap();
    let first = reconcile::run(r.path()).unwrap().render();
    assert_eq!(first, reconcile::run(r.path()).unwrap().render());
    let value = json::parse_unique(&first).unwrap();
    assert_eq!(value.items_of("per_donor").len(), 68);
    assert_eq!(
        value.at("reviewed_source_oss.included"),
        Some(&Value::Int(3))
    );
    assert_eq!(
        value.at("reviewed_source_oss.unresolved"),
        Some(&Value::Int(65))
    );
    assert_eq!(value.at("reviewed_source_oss.total"), Some(&Value::Null));
    assert_eq!(
        value.at("reviewed_source_oss.verified_extinct"),
        Some(&Value::Null)
    );
    assert_eq!(value.get("source_audit"), Some(&independent));
    assert_eq!(value.get("new_assimilation"), Some(&Value::Int(0)));
    assert_eq!(value.get("new_extinction"), Some(&Value::Int(0)));
    assert_eq!(
        value.get("lifecycle_gates_modified"),
        Some(&Value::Bool(false))
    );
    assert_eq!(value.str("canonical_registry_binding"), Some("PENDING"));
    let crawlee = row(&value, CRAWLEE);
    for (key, expected) in [
        ("source_id", "ec1:c90af235a2857b25e3bbeb288f16fcd4"),
        ("origin", "https://github.com/apify/crawlee"),
        ("review.status", "SOURCE_REVIEWED_OSS_INCLUSION"),
        ("review.original_record_id", "#12"),
        (
            "review.original_provenance",
            "research/commerce/donors/registry.json##12",
        ),
        (
            "review.upstream_revision",
            "438e3419626bd070f8984566bfb86ab9355f55d6",
        ),
        ("review.declared_license", "Apache-2.0"),
        ("review.historical_license_receipt_status", "UNVERIFIED"),
        ("review.retained_root_license.path", "LICENSE.md"),
        (
            "review.retained_root_license.sha256",
            "sha256:37c9b417332af22e146d381766f284649483fb6ece17ca566584bf6adcf89135",
        ),
        (
            "review.retained_root_license.blob",
            "8effb33b6996f6db058161cfe48b01e1f614e726",
        ),
        ("review.canonical_registry_binding", "PENDING"),
    ] {
        assert_eq!(
            crawlee.at(key).and_then(Value::as_str),
            Some(expected),
            "{key}"
        );
    }
    for (key, expected) in [
        ("review.inventory_files", 2992),
        ("review.first_party_source_files", 1347),
        ("review.retained_root_license.bytes", 11355),
    ] {
        assert_eq!(crawlee.at(key), Some(&Value::Int(expected)), "{key}");
    }
    assert!(crawlee
        .at("review.current_copying_authorization")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("UNKNOWN"));
    let identity = json::parse_unique(&r.read(&format!("{CRAWLEE_CENSUS}/identity.json"))).unwrap();
    assert_eq!(identity.str("donor_type"), Some("UNCLASSIFIED"));
    let summary = json::parse_unique(&r.read(&format!("{CRAWLEE_CENSUS}/summary.json"))).unwrap();
    assert_eq!(summary.str("status"), Some("CENSUS_PARTIAL"));
    assert_eq!(summary.str("semantic_review"), Some("PENDING"));
    assert_eq!(summary.get("source_parsed"), Some(&Value::Int(1347)));
    let lifecycle =
        json::parse_unique(&r.read("research/commerce/donor-lifecycle-review.json")).unwrap();
    let history = lifecycle
        .items_of("donors")
        .iter()
        .find(|d| d.str("donor_id") == Some(CRAWLEE))
        .unwrap();
    for key in ["full_semantic_census", "full_absorption", "extinct"] {
        assert_eq!(history.get(key), Some(&Value::Bool(false)), "{key}");
    }
    let declaration = r.declaration();
    let donor = declaration
        .donors
        .iter()
        .find(|d| d.key == CRAWLEE)
        .unwrap();
    let remaining = donor
        .capabilities
        .iter()
        .find(|c| c.key == "crawlee.remaining-crawler-framework")
        .unwrap();
    assert!(!remaining.required);
    assert!(matches!(
        &remaining.relevance,
        ecdev_governance::declare::Relevance::Unresolved
    ));
    let original = r
        .read(REGISTRY_RECORDS)
        .lines()
        .map(|line| json::parse_unique(line).unwrap())
        .find(|record| record.str("id") == Some("#12"))
        .unwrap();
    assert_eq!(
        content_digest(original.text("legacy").as_bytes()),
        "sha256:c22f17e4f41bddef1b7dde643a5f4cca945fe19dd92894b6414a1c3ea39c42a6"
    );
    assert_eq!(tree(r.path()), before);

    // The ledger is an additional classification view. The fixture deliberately
    // contains no executable production caller and supplies no fresh proof credit.
    select_reviews(&r, &[PRICE, EXTRUCT]);
    let two = report(&r);
    assert_eq!(two.at("reviewed_source_oss.included"), Some(&Value::Int(2)));
    assert_eq!(
        row(&two, CRAWLEE).at("review.status"),
        Some(&Value::Str("UNRESOLVED".into()))
    );
    for key in [
        "donors",
        "mixed_declared_cohort",
        "active_extinction_gate",
        "audited_oss",
    ] {
        assert_eq!(
            two.at(&format!("source_audit.{key}")),
            independent.get(key),
            "{key}"
        );
    }
    for (path, bytes) in before {
        if path != LEDGER {
            assert_eq!(std::fs::read(r.root.join(&path)).unwrap(), bytes, "{path}");
        }
    }
}

#[test]
fn crawlee_receipt_failures_refuse_the_whole_three_review_report_without_writes() {
    for path in [
        format!("{CRAWLEE_CENSUS}/identity.json"),
        format!("{CRAWLEE_CENSUS}/license.json"),
        format!("{CRAWLEE_CENSUS}/files.jsonl"),
        format!("{CRAWLEE_CENSUS}/tree.json"),
        format!("{CRAWLEE_CENSUS}/summary.json"),
        "research/commerce/donor-lifecycle-review.json".into(),
    ] {
        let r = three_source_fixture("reconcile-third-missing");
        r.remove(&path);
        refused_without_writes(&r, &path);
        let r = three_source_fixture("reconcile-third-tampered");
        r.write(&path, &(r.read(&path) + " \n"));
        refused_with(&r, &path, "source review sha256 mismatch");
    }
    let r = three_source_fixture("reconcile-third-license-rebound");
    rebind_receipt(&r, &format!("{CRAWLEE_CENSUS}/license.json"), |license| {
        let Value::Array(files) = field_mut(license, "files") else {
            panic!("licence files must be an array")
        };
        let changed = format!("{}\nSubstituted terms.\n", files[0].text("text"));
        *field_mut(&mut files[0], "text") = Value::Str(changed.clone());
        *field_mut(&mut files[0], "sha256") =
            Value::Str(content_digest(changed.as_bytes())[7..].into());
    });
    refused_with(
        &r,
        "Crawlee licence self-hash",
        "retained license text mismatch",
    );
    let r = three_source_fixture("reconcile-third-summary-rebound");
    rebind_receipt(&r, &format!("{CRAWLEE_CENSUS}/summary.json"), |summary| {
        *field_mut(summary, "source_parsed") = Value::Int(1346);
    });
    refused_with(
        &r,
        "Crawlee parse count",
        "review closed source census counts or parse assertions disagree",
    );
    let r = three_source_fixture("reconcile-third-pin-rebound");
    rebind_receipt(&r, &format!("{CRAWLEE_CENSUS}/identity.json"), |identity| {
        *field_mut(identity, "commit_sha") =
            Value::Str("0000000000000000000000000000000000000000".into());
    });
    refused_with(
        &r,
        "Crawlee upstream revision",
        "source review commit_sha mismatch",
    );
}
