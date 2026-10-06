//! Deterministic knowledge compaction.
//!
//! ```text
//! declarations + census + evidence + history + extracted document facts
//!        │  COMPACTOR
//!        ├── one folded history batch        .ecdev/history/<digest>.ecg     (authoritative)
//!        ├── one folded knowledge batch      .ecdev/knowledge/<digest>.ecg   (authoritative)
//!        ├── evidence without stale records  .ecdev/evidence/<digest>.ecg    (authoritative)
//!        ├── retired evidence history        .ecdev/evidence/<digest>.ecg    (authoritative: records
//!        │                                   of undeclared proofs, with provenance; never evidence)
//!        ├── the knowledge capsule           target/ecdev-governance/capsule.ecg        (generated index)
//!        └── the human view                  target/ecdev-governance/VIEW.md            (generated view)
//! ```
//!
//! Generated outputs are never read back as truth: deleting them changes nothing.

pub mod codec;
pub mod facts;
pub mod history;
pub mod outline;
pub mod view;

use crate::digest::{hex, sha256};
use std::path::Path;

pub const GENERATED_DIR: &str = "target/ecdev-governance";
pub const CAPSULE_FILE: &str = "target/ecdev-governance/index.ecg";
pub const VIEW_FILE: &str = "target/ecdev-governance/VIEW.md";
pub const CAPSULE_TAG: u8 = 5;

/// Writes `bytes` to `<dir>/<first 32 hex of sha256>.ecg`; returns the repository path.
pub fn write_addressed(root: &Path, dir: &str, bytes: &[u8]) -> std::io::Result<String> {
    let name = format!("{}.{}", &hex(&sha256(bytes))[..32], codec::EXTENSION);
    std::fs::create_dir_all(root.join(dir))?;
    std::fs::write(root.join(dir).join(&name), bytes)?;
    Ok(format!("{dir}/{name}"))
}

/// Reads every content-addressed file of `dir`, sorted by name, verifying each address.
pub fn read_addressed(
    root: &Path,
    dir: &str,
    unreadable: &mut Vec<String>,
) -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(&format!(".{}", codec::EXTENSION)))
        .collect();
    names.sort();
    let mut out = Vec::new();
    for n in names {
        match std::fs::read(root.join(dir).join(&n)) {
            Ok(b) => {
                if format!("{}.{}", &hex(&sha256(&b))[..32], codec::EXTENSION) == n {
                    out.push((n, b));
                } else {
                    unreadable.push(format!("{dir}/{n}: name is not the digest of its content"));
                }
            }
            Err(e) => unreadable.push(format!("{dir}/{n}: {e}")),
        }
    }
    out
}

/// The generated knowledge capsule: graph, current facts and metrics in one index.
pub fn encode_capsule(graph: &[u8], facts: &[u8], metrics: &[(String, String)]) -> Vec<u8> {
    let mut e = codec::Encoder::new(CAPSULE_TAG);
    e.str(&crate::protocol::schema_identity())
        .bytes(graph)
        .bytes(facts)
        .u64(metrics.len() as u64);
    for (k, v) in metrics {
        e.str(k).str(v);
    }
    e.finish()
}
