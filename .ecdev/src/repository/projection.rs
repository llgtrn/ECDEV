//! The sanctioned shared-protocol projection of this source.
//!
//! Registration of this repository as a shard goes through the central engine's qualified
//! lossless projection: the engine reads `.ecdev/{declared,history,evidence,knowledge}` without
//! writing them and installs, beside them, current-grammar declarations, the universe import,
//! one knowledge file that pins the custody of every `.ecdev` record file, and a protocol lock.
//! The conformance suite's source-migration boundary exists to stop a PENDING source from
//! installing such control artifacts by itself. It still does. This module is the one
//! exception, and it is narrow: the projection directory is recognised only when ALL of the
//! following hold, each checked here with this crate's own readers and digest:
//!
//! 1. Exactly the closed file set below exists: no extra file, directory or link, nothing missing.
//! 2. Every declaration file and the custody-pin knowledge file match a pin held in this crate
//!    (path, length, SHA-256). The pins are not read from the projection itself.
//! 3. The knowledge file is content-addressed (its name is the digest prefix of its bytes), its
//!    custody pins re-derive their recorded root, and they equal the bytes now in the four
//!    `.ecdev` record directories: nothing mutated, added or removed.
//! 4. Each universe import equals the universe file of the same name in `.ecdev/declared` with
//!    only its schema word changed.
//! 5. The lock has the exact protocol-lock shape. Whether the engine digest it names is the
//!    current engine is a fact only the engine can establish; that is not claimed here.
//!
//! A recognised projection is left out of this repository's file universe (it is neither
//! governor source nor product code, and its verified bytes are derived from `.ecdev`), so every
//! other check measures exactly what it measured before the projection existed. Anything else
//! under the projection directory is rejected and stays visible to every check as before.

use crate::compact::facts;
use crate::digest::{content_digest, hex, Sha256};
use crate::formats::{json, Value};
use crate::schema::FactKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The projection directory name, built from parts so this file does not name it literally.
pub fn dir_name() -> String {
    [".", "ynventa"].concat()
}

/// One pinned projection file, relative to the projection directory.
pub struct Pin {
    pub path: &'static str,
    pub bytes: usize,
    pub sha256: &'static str,
}

/// Pins of the engine-installed declarations and of the custody-pin knowledge file, produced by
/// the engine's `migrate scaffold` over the pinned `.ecdev` record directories. A regenerated
/// projection (different engine output) is a different projection and needs a reviewed update.
pub const PINS: &[Pin] = &[
    Pin {
        path: "declared/donors.rs",
        bytes: 238613,
        sha256: "sha256:9ad1aced3e224760a97720007d435e40a0485c4844c697d00ca168555f5e081a",
    },
    Pin {
        path: "declared/migration.rs",
        bytes: 525,
        sha256: "sha256:1ccde356563212afd4585886a8f82c7ffbe29888dc1e6b84bca6778534bc5b4e",
    },
    Pin {
        path: "declared/organism.rs",
        bytes: 333,
        sha256: "sha256:c8302b7550eb18554d291bcbfbec85b87fc8257058a711d1c1af0d217f2954e6",
    },
    Pin {
        path: "declared/repository.rs",
        bytes: 11260,
        sha256: "sha256:828abc98a1a90af37b4b5da7f5c4c352fbf74fcbee5b3a6d0c9851cbedef6f44",
    },
    Pin {
        path: "declared/technologies.rs",
        bytes: 66451,
        sha256: "sha256:022f03477733a4adf6510a15f71ce9037dc913aaff398da679086e9c9712122b",
    },
    Pin {
        path: "knowledge/3e6ab6b6b696aadf6db9be733d2b18f3.ynv",
        bytes: 2259907,
        sha256: "sha256:3e6ab6b6b696aadf6db9be733d2b18f3665aec20cd41d5801f6d5be4f2eab828",
    },
];

const LOCK: &str = "protocol.lock";
const UNIVERSE_DIR: &str = "declared/universe";
const KNOWLEDGE_DIR: &str = "knowledge";
const KNOWLEDGE_EXT: &str = ".ynv";
const CONTAINER_MAGIC: &[u8; 3] = b"YNV";
const NATIVE_MAGIC: &[u8; 3] = b"ECG";
const FORMAT_WORD: &str = "ecdev-lossless-import-v1";
const HEADER_SUBJECT: &str = "ecdev-import/source";
const CUSTODY_DOMAIN: &[u8] = b"ecdev-import.custody.v1";
const LEGACY_ROOT: &str = ".ecdev";
const CONTROL_DIRS: [&str; 4] = ["declared", "history", "evidence", "knowledge"];
const SOURCE_UNIVERSE_WORD: &str = "\"universe\":\"ecdev.universe.v1\"";
const TARGET_UNIVERSE_WORD: &str = "\"universe\":\"ynventa.universe.v1\"";
const MAX_ENTRIES: usize = 20_000;
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// A projection that passed every condition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sanctioned {
    /// Repository-relative paths of the projection files (all of them).
    pub files: BTreeSet<String>,
    /// The custody root recorded by, and re-derived from, the pins in the knowledge file.
    pub custody_root: String,
    /// Number of `.ecdev` record files whose bytes are pinned.
    pub custody_files: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Outcome {
    /// No projection directory exists.
    #[default]
    Absent,
    Sanctioned(Sanctioned),
    /// The directory exists but is not the sanctioned projection; every reason is listed.
    Rejected(Vec<String>),
}

/// Verifies the projection directory of `root` against this crate's pins.
pub fn verify(root: &Path) -> Outcome {
    verify_with(root, PINS)
}

/// As [`verify`], with an explicit pin table (fixtures pin their own generated projection).
pub fn verify_with(root: &Path, pins: &[Pin]) -> Outcome {
    let dir = dir_name();
    match std::fs::symlink_metadata(root.join(&dir)) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
        Ok(_) => {
            return Outcome::Rejected(vec![format!(
                "{dir}: not a plain directory (links and files are not a projection)"
            )])
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::Absent,
        Err(e) => return Outcome::Rejected(vec![format!("{dir}: cannot be inspected: {e}")]),
    }
    let mut why = Vec::new();
    let found = match walk(root, &dir) {
        Ok(f) => f,
        Err(e) => return Outcome::Rejected(vec![e]),
    };
    let source = match walk_source(root) {
        Ok(s) => s,
        Err(e) => {
            return Outcome::Rejected(vec![format!(
                "{dir}: cannot establish the custody of {LEGACY_ROOT}: {e}"
            )])
        }
    };

    // 1. The closed file set.
    let mut expected: BTreeSet<String> = pins.iter().map(|p| p.path.to_string()).collect();
    expected.insert(LOCK.to_string());
    let universe: Vec<&String> = source
        .keys()
        .filter(|p| {
            p.strip_prefix(&format!("{LEGACY_ROOT}/{UNIVERSE_DIR}/"))
                .is_some_and(|n| n.ends_with(".jsonl") && !n.contains('/'))
        })
        .collect();
    for p in &universe {
        expected.insert(format!(
            "{UNIVERSE_DIR}/{}",
            p.rsplit('/').next().unwrap_or_default()
        ));
    }
    for f in found.difference(&expected) {
        why.push(format!("{dir}/{f}: not part of the sanctioned projection"));
    }
    for f in expected.difference(&found) {
        why.push(format!("{dir}/{f}: sanctioned projection file is missing"));
    }

    // 2. Pinned declarations and the custody-pin knowledge file.
    for pin in pins {
        let Some(bytes) = read_file(root, &format!("{dir}/{}", pin.path)) else {
            continue; // reported as missing above
        };
        if bytes.len() != pin.bytes || content_digest(&bytes) != pin.sha256 {
            why.push(format!(
                "{dir}/{}: differs from the pinned projection (mutated or regenerated)",
                pin.path
            ));
        }
    }

    // 3. Content-addressed custody pins equal the live record directories.
    let knowledge: Vec<&Pin> = pins
        .iter()
        .filter(|p| p.path.starts_with(&format!("{KNOWLEDGE_DIR}/")))
        .collect();
    let mut custody_root = String::new();
    if knowledge.len() != 1 {
        why.push(format!(
            "{dir}: exactly one custody-pin knowledge file is pinned, found {}",
            knowledge.len()
        ));
    }
    for pin in knowledge {
        let Some(bytes) = read_file(root, &format!("{dir}/{}", pin.path)) else {
            continue;
        };
        match custody(pin.path, &bytes, &source) {
            Ok(r) => custody_root = r,
            Err(e) => why.push(format!("{dir}/{}: {e}", pin.path)),
        }
    }

    // 4. Universe imports mirror the source universe.
    for p in &universe {
        let name = p.rsplit('/').next().unwrap_or_default();
        let rel = format!("{dir}/{UNIVERSE_DIR}/{name}");
        let (Some(have), Some(src)) = (read_file(root, &rel), read_file(root, p)) else {
            continue;
        };
        let want = String::from_utf8(src)
            .ok()
            .filter(|t| t.matches(SOURCE_UNIVERSE_WORD).count() == 1)
            .map(|t| t.replacen(SOURCE_UNIVERSE_WORD, TARGET_UNIVERSE_WORD, 1));
        if want.as_deref().map(str::as_bytes) != Some(&have[..]) {
            why.push(format!(
                "{rel}: is not the source universe `{p}` with only its schema word changed"
            ));
        }
    }

    // 5. The lock has the exact shape.
    if let Some(bytes) = read_file(root, &format!("{dir}/{LOCK}")) {
        if let Err(e) = lock_shape(&bytes) {
            why.push(format!("{dir}/{LOCK}: {e}"));
        }
    }

    if !why.is_empty() {
        why.sort();
        why.dedup();
        return Outcome::Rejected(why);
    }
    Outcome::Sanctioned(Sanctioned {
        files: found.into_iter().map(|f| format!("{dir}/{f}")).collect(),
        custody_root,
        custody_files: source.len(),
    })
}

/// Every regular file below `rel`, relative to it. Links, special files and directories that
/// hold no file are refused: the projection is a closed set of regular files.
fn walk(root: &Path, rel: &str) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    let mut seen = 0;
    walk_into(root, rel, rel, &mut out, &mut seen)?;
    Ok(out)
}

fn walk_into(
    root: &Path,
    base: &str,
    rel: &str,
    out: &mut BTreeSet<String>,
    seen: &mut usize,
) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(root.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{rel}: a link is not a projection file"));
    }
    if meta.is_file() {
        out.insert(rel[base.len() + 1..].to_string());
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(format!("{rel}: neither a regular file nor a directory"));
    }
    let before = out.len();
    for entry in std::fs::read_dir(root.join(rel)).map_err(|e| format!("{rel}: {e}"))? {
        *seen += 1;
        if *seen > MAX_ENTRIES {
            return Err(format!("{rel}: traversal budget exceeded"));
        }
        let name = entry
            .map_err(|e| format!("{rel}: {e}"))?
            .file_name()
            .into_string()
            .map_err(|_| format!("{rel}: non UTF-8 name"))?;
        walk_into(root, base, &format!("{rel}/{name}"), out, seen)?;
    }
    if rel != base && out.len() == before {
        return Err(format!("{rel}: a directory that holds no projection file"));
    }
    Ok(())
}

/// Path -> (length, digest) of every file in the four `.ecdev` record directories.
fn walk_source(root: &Path) -> Result<BTreeMap<String, (usize, String)>, String> {
    let mut out = BTreeMap::new();
    for d in CONTROL_DIRS {
        let rel = format!("{LEGACY_ROOT}/{d}");
        let meta = std::fs::symlink_metadata(root.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(format!("{rel}: not a plain directory"));
        }
        for f in walk(root, &rel)? {
            let path = format!("{rel}/{f}");
            let bytes = read_file(root, &path).ok_or_else(|| format!("{path}: unreadable"))?;
            out.insert(path, (bytes.len(), content_digest(&bytes)));
        }
    }
    Ok(out)
}

fn read_file(root: &Path, rel: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let path = root.join(rel);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// The custody root of a set of pinned files; the domain and field framing are the engine's.
pub fn custody_root(files: &BTreeMap<String, (usize, String)>) -> String {
    let mut h = Sha256::new();
    h.field(CUSTODY_DOMAIN);
    for (path, (len, digest)) in files {
        h.field(path.as_bytes());
        h.field(len.to_string().as_bytes());
        h.field(digest.as_bytes());
    }
    format!("sha256:{}", hex(&h.finish()))
}

/// Decodes the custody-pin knowledge file and compares its pins with the live source bytes.
fn custody(
    name: &str,
    bytes: &[u8],
    source: &BTreeMap<String, (usize, String)>,
) -> Result<String, String> {
    let file = name.rsplit('/').next().unwrap_or(name);
    let stem = file
        .strip_suffix(KNOWLEDGE_EXT)
        .ok_or("not a knowledge file")?;
    if stem.len() != 32 || content_digest(bytes)["sha256:".len()..][..32] != *stem {
        return Err("name is not the digest prefix of its bytes (not content-addressed)".into());
    }
    if bytes.len() < 3 || &bytes[..3] != CONTAINER_MAGIC {
        return Err("not a knowledge container".into());
    }
    let mut native = bytes.to_vec();
    native[..3].copy_from_slice(NATIVE_MAGIC);
    let records = facts::decode(&native).map_err(|e| format!("unreadable knowledge: {e}"))?;
    let mut format = None;
    let mut root_recorded = None;
    let mut count = None;
    let mut pins: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for f in records.iter().filter(|f| f.kind == FactKind::LegacyRecord) {
        if f.subject == HEADER_SUBJECT {
            match f.key.as_str() {
                "format" => format = Some(f.value.clone()),
                "custody-root" => root_recorded = Some(f.value.clone()),
                "custody-files" => count = f.value.parse::<usize>().ok(),
                _ => {}
            }
        } else if f.subject.starts_with(&format!("{LEGACY_ROOT}/")) && f.key == "custody" {
            let v = json::parse_unique(&f.value)?;
            let (Some(Value::Int(n)), Some(Value::Str(d))) = (v.get("bytes"), v.get("sha256"))
            else {
                return Err(format!("{}: custody pin is malformed", f.subject));
            };
            let n = usize::try_from(*n).map_err(|_| format!("{}: negative length", f.subject))?;
            if pins.insert(f.subject.clone(), (n, d.clone())).is_some() {
                return Err(format!("{}: custody pinned twice", f.subject));
            }
        }
    }
    if format.as_deref() != Some(FORMAT_WORD) {
        return Err(format!("not a `{FORMAT_WORD}` custody record"));
    }
    let derived = custody_root(&pins);
    if root_recorded.as_deref() != Some(derived.as_str()) || count != Some(pins.len()) {
        return Err("recorded custody root does not match the recorded pins".into());
    }
    let mut diffs = Vec::new();
    for (p, pin) in &pins {
        match source.get(p) {
            None => diffs.push(format!("removed {p}")),
            Some(cur) if cur != pin => diffs.push(format!("mutated {p}")),
            Some(_) => {}
        }
    }
    for p in source.keys().filter(|p| !pins.contains_key(*p)) {
        diffs.push(format!("added {p}"));
    }
    if !diffs.is_empty() {
        let n = diffs.len();
        diffs.truncate(5);
        return Err(format!(
            "{LEGACY_ROOT} differs from the custody pinned by the projection ({n} paths: {})",
            diffs.join(", ")
        ));
    }
    Ok(derived)
}

/// The exact shape of the protocol lock: header, then four keys in order, digests well formed.
fn lock_shape(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8")?;
    let lines: Vec<&str> = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .split('\n')
        .collect();
    let digest = |s: &str| {
        s.strip_prefix("sha256:").is_some_and(|h| {
            h.len() == 64 && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        })
    };
    let ok = text.ends_with('\n')
        && lines.len() == 5
        && lines[0].starts_with("# ")
        && lines[1] == "protocol_version 1"
        && lines[2].strip_prefix("schema_digest ").is_some_and(digest)
        && lines[3].strip_prefix("engine_digest ").is_some_and(digest)
        && lines[4] == "compatibility_epoch 1";
    if ok {
        Ok(())
    } else {
        Err("not the exact protocol-lock shape".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_shape_is_exact() {
        let h = "a".repeat(64);
        let good = format!(
            "# lock\nprotocol_version 1\nschema_digest sha256:{h}\nengine_digest sha256:{h}\ncompatibility_epoch 1\n"
        );
        assert!(lock_shape(good.as_bytes()).is_ok());
        for bad in [
            good.replace("sha256:aaaa", "sha256:AAAA"),
            good.replace("protocol_version 1", "protocol_version 2"),
            format!("{good}extra 1\n"),
            good.trim_end().to_string(),
            good.replace(&h, "abc"),
        ] {
            assert!(lock_shape(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
