//! The engine-installed projection is recognised only when it is the pinned, closed,
//! custody-consistent file set; everything else about the source-migration boundary is unchanged.
//!
//! Fixtures copy the real governance record directories and the real projection into a scratch
//! root, so the positive cases use the production pins and the negative cases mutate exactly one
//! thing at a time.

use ecdev_governance::repository::files::Files;
use ecdev_governance::repository::projection::{dir_name, verify, Outcome};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_tree(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    /// The real record directories and the real projection, outside any Git directory.
    fn new(name: &str) -> Fixture {
        let real = ecdev_governance::default_root();
        let root = std::env::temp_dir().join(format!(
            "ecdev-projection-{name}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["declared", "history", "evidence", "knowledge"] {
            copy_tree(&real.join(".ecdev").join(d), &root.join(".ecdev").join(d));
        }
        copy_tree(&real.join(dir_name()), &root.join(dir_name()));
        Fixture { root }
    }
    fn proj(&self, rel: &str) -> PathBuf {
        self.root.join(dir_name()).join(rel)
    }
    fn outcome(&self) -> Outcome {
        verify(&self.root)
    }
    fn rejected(&self) -> String {
        match self.outcome() {
            Outcome::Rejected(why) => why.join("\n"),
            other => panic!("expected a rejection, got {other:?}"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn knowledge_file(f: &Fixture) -> PathBuf {
    let dir = f.proj("knowledge");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .next()
        .unwrap()
}

fn first_file(dir: &Path) -> PathBuf {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    v.remove(0)
}

#[test]
fn the_installed_projection_of_this_repository_is_sanctioned() {
    match verify(&ecdev_governance::default_root()) {
        Outcome::Sanctioned(s) => {
            assert!(s.custody_files > 0 && s.custody_root.starts_with("sha256:"));
            assert!(s.files.contains(&format!("{}/protocol.lock", dir_name())));
            assert!(s.files.iter().all(|f| f.starts_with(&dir_name())));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_sanctioned_projection_leaves_the_file_universe_and_the_boundary_passes() {
    let f = Fixture::new("positive");
    assert!(matches!(f.outcome(), Outcome::Sanctioned(_)));
    let files = Files::scan(&f.root).unwrap();
    assert!(
        files.paths.iter().all(|p| !p.starts_with(&dir_name())),
        "verified projection files are not governor or product source"
    );
    assert!(matches!(files.projection, Outcome::Sanctioned(_)));
    let a = ecdev_governance::assess(&f.root).unwrap();
    for id in [
        "governance.source_migration_boundary",
        "governance.no_external_authority",
    ] {
        let c = ecdev_governance::conformance::checks(&a)
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        // The fixture holds only the record directories, so its own unrelated findings may
        // exist; the boundary itself must hold and must say what it verified.
        if id == "governance.source_migration_boundary" {
            assert!(c.pass, "{}", c.detail);
            assert!(c.detail.contains("ENROLLMENT_NOT_ESTABLISHED"));
            assert!(c.detail.contains("matches its pins"));
        } else {
            assert!(c.pass, "{}", c.detail);
        }
    }
    assert!(ecdev_governance::conformance::source_migration_violations(&a).is_empty());
}

#[test]
fn a_nested_projection_directory_is_never_sanctioned() {
    let f = Fixture::new("nested");
    let nested = f.root.join("tools").join(dir_name());
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("captured.txt"), "control input").unwrap();
    let files = Files::scan(&f.root).unwrap();
    assert!(files
        .paths
        .iter()
        .any(|p| p == &format!("tools/{}/captured.txt", dir_name())));
    let a = ecdev_governance::assess(&f.root).unwrap();
    let v = ecdev_governance::conformance::source_migration_violations(&a);
    assert!(
        v.iter().any(|m| m.contains("captured.txt")),
        "a second projection directory stays a violation: {v:?}"
    );
}

#[test]
fn a_mutated_declaration_is_rejected_and_every_old_violation_returns() {
    let f = Fixture::new("mutated-declaration");
    let p = f.proj("declared/repository.rs");
    let mut text = std::fs::read_to_string(&p).unwrap();
    text.push_str("// forged\n");
    std::fs::write(&p, text).unwrap();
    assert!(f.rejected().contains("declared/repository.rs"));
    let files = Files::scan(&f.root).unwrap();
    assert!(
        files
            .paths
            .iter()
            .any(|p| p.starts_with(&dir_name()) && p.ends_with("repository.rs")),
        "a rejected projection stays visible to every check"
    );
    let a = ecdev_governance::assess(&f.root).unwrap();
    let v = ecdev_governance::conformance::source_migration_violations(&a);
    assert!(v.iter().any(|m| m.contains("not admitted")), "{v:?}");
    assert!(
        v.iter()
            .any(|m| m.contains("pending source cannot install protocol control artifacts")),
        "{v:?}"
    );
}

#[test]
fn an_unknown_extra_file_is_rejected() {
    let f = Fixture::new("extra-file");
    std::fs::write(f.proj("declared/extra.rs"), "fn main() {}\n").unwrap();
    assert!(f.rejected().contains("declared/extra.rs"));
    let g = Fixture::new("extra-root-file");
    std::fs::write(g.proj("README.md"), "hello\n").unwrap();
    assert!(g.rejected().contains("README.md"));
    let h = Fixture::new("extra-dir");
    std::fs::create_dir(h.proj("src")).unwrap();
    assert!(h.rejected().contains("src"));
}

#[test]
fn a_missing_pinned_or_required_file_is_rejected() {
    for rel in [
        "declared/donors.rs",
        "declared/organism.rs",
        "protocol.lock",
    ] {
        let f = Fixture::new("missing");
        std::fs::remove_file(f.proj(rel)).unwrap();
        assert!(f.rejected().contains(rel), "{rel}");
    }
    let f = Fixture::new("missing-knowledge");
    std::fs::remove_file(knowledge_file(&f)).unwrap();
    assert!(f.rejected().contains("knowledge"));
    let g = Fixture::new("missing-universe");
    let u = first_file(&g.proj("declared/universe"));
    std::fs::remove_file(u).unwrap();
    assert!(g.rejected().contains("universe"));
}

#[test]
fn a_forged_custody_record_is_rejected() {
    let f = Fixture::new("forged-knowledge");
    let k = knowledge_file(&f);
    let mut bytes = std::fs::read(&k).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&k, bytes).unwrap();
    let why = f.rejected();
    assert!(why.contains("differs from the pinned projection"), "{why}");
    assert!(why.contains("not content-addressed"), "{why}");
}

#[test]
fn a_mutated_added_or_removed_record_is_a_custody_drift() {
    let f = Fixture::new("record-mutated");
    let rec = first_file(&f.root.join(".ecdev/evidence"));
    let mut bytes = std::fs::read(&rec).unwrap();
    bytes.push(0);
    std::fs::write(&rec, bytes).unwrap();
    let why = f.rejected();
    assert!(why.contains("mutated .ecdev/evidence/"), "{why}");

    let g = Fixture::new("record-added");
    std::fs::write(g.root.join(".ecdev/history/added.ecg"), b"x").unwrap();
    assert!(g.rejected().contains("added .ecdev/history/added.ecg"));

    let h = Fixture::new("record-removed");
    std::fs::remove_file(first_file(&h.root.join(".ecdev/knowledge"))).unwrap();
    assert!(h.rejected().contains("removed .ecdev/knowledge/"));
}

#[test]
fn a_hand_written_or_malformed_lock_is_rejected() {
    let h = "0".repeat(64);
    for lock in [
        String::new(),
        "protocol_version 1\n".to_string(),
        format!(
            "# lock\nprotocol_version 1\nschema_digest sha256:{h}\nengine_digest sha256:{h}\ncompatibility_epoch 1\nextra yes\n"
        ),
        format!(
            "# lock\nprotocol_version 2\nschema_digest sha256:{h}\nengine_digest sha256:{h}\ncompatibility_epoch 1\n"
        ),
        format!(
            "# lock\nprotocol_version 1\nschema_digest sha256:short\nengine_digest sha256:{h}\ncompatibility_epoch 1\n"
        ),
    ] {
        let f = Fixture::new("lock");
        std::fs::write(f.proj("protocol.lock"), lock).unwrap();
        assert!(f.rejected().contains("protocol.lock"));
    }
}

#[test]
fn a_changed_universe_import_is_rejected() {
    let f = Fixture::new("universe");
    let u = first_file(&f.proj("declared/universe"));
    let text = std::fs::read_to_string(&u).unwrap();
    std::fs::write(&u, text.replacen("\"records\":34", "\"records\":35", 1)).unwrap();
    assert!(f.rejected().contains("source universe"));
}

#[cfg(unix)]
#[test]
fn links_are_never_a_projection() {
    let f = Fixture::new("link-file");
    let p = f.proj("declared/organism.rs");
    std::fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(f.proj("declared/migration.rs"), &p).unwrap();
    assert!(f.rejected().contains("link"));

    let g = Fixture::new("link-dir");
    let moved = g.root.join("moved");
    std::fs::rename(g.root.join(dir_name()), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, g.root.join(dir_name())).unwrap();
    assert!(g.rejected().contains("not a plain directory"));
}

#[test]
fn without_a_projection_directory_nothing_changes() {
    let f = Fixture::new("absent");
    std::fs::remove_dir_all(f.root.join(dir_name())).unwrap();
    assert_eq!(f.outcome(), Outcome::Absent);
    let files = Files::scan(&f.root).unwrap();
    assert_eq!(files.projection, Outcome::Absent);
    let a = ecdev_governance::assess(&f.root).unwrap();
    assert!(ecdev_governance::conformance::source_migration_violations(&a).is_empty());
    assert!(ecdev_governance::conformance::checks(&a)
        .iter()
        .find(|c| c.id == "governance.source_migration_boundary")
        .unwrap()
        .detail
        .contains("no shared projection or protocol control artifacts installed"));
}
