//! Read-only transport of source governance bytes. This is a capture, never enrollment,
//! a canonical declaration, proof execution, or an attestation of repository-history completeness.

use crate::compact::{codec::Decoder, facts, history::Batch};
use crate::declare::{self, Declaration};
use crate::digest::{content_digest, hex, unhex};
use crate::evidence;
use crate::formats::{json, json::Json, Value};
use crate::repository::files::head_commit;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

pub const FORMAT: &str = "ecdev-source-capture-v1";
const BINDING: &str = "CAPTURED_BYTES; HEAD_OBSERVED_ONLY; NOT_ENROLLED; NOT_REVERIFIED";
const DIRS: &[&str] = &["declared", "history", "evidence", "knowledge"];
const MAX_FILE: usize = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The observed HEAD, not a claim that these working-tree bytes belong to that commit.
    pub observed_head: String,
    /// All original paths and bytes, including universe inputs and retired evidence.
    pub files: BTreeMap<String, Vec<u8>>,
}

fn allowed(path: &str) -> bool {
    !path
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
        && !path.contains('\\')
        && path.split('/').count() <= 32
        && DIRS
            .iter()
            .any(|d| path.starts_with(&format!(".ecdev/{d}/")))
}

fn directory(path: &Path) -> Result<(), String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err(format!(
            "{}: expected a directory, not a link",
            path.display()
        ));
    }
    Ok(())
}

fn collect(root: &Path, rel: &str, out: &mut Capture, total: &mut usize) -> Result<(), String> {
    if rel.split('/').count() > 32 {
        return Err("source capture directory depth exceeds budget".into());
    }
    directory(&root.join(rel))?;
    let entries = std::fs::read_dir(root.join(rel)).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non UTF-8 source path")?;
        let path = format!("{rel}/{name}");
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect(root, &path, out, total)?;
        } else if kind.is_file() {
            if out.files.len() >= MAX_FILES {
                return Err("source capture file count exceeds budget".into());
            }
            let mut bytes = Vec::new();
            std::fs::File::open(entry.path())
                .map_err(|e| e.to_string())?
                .take((MAX_FILE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            *total += bytes.len();
            if bytes.len() > MAX_FILE || *total > MAX_TOTAL {
                return Err(format!("{path}: source capture byte budget exceeded"));
            }
            out.files.insert(path, bytes);
        } else {
            return Err(format!(
                "{path}: source capture refuses links and special files"
            ));
        }
    }
    Ok(())
}

/// Capture only the four source-state directories; no source program or Cargo command runs.
/// Files need not be committed: the binding deliberately makes no commit-content claim.
pub fn capture(root: &Path) -> Result<Capture, String> {
    directory(&root.join(".ecdev"))?;
    let mut out = Capture {
        observed_head: head_commit(root).unwrap_or_default(),
        files: BTreeMap::new(),
    };
    let mut total = 0;
    for dir in DIRS {
        collect(root, &format!(".ecdev/{dir}"), &mut out, &mut total)?;
    }
    out.validate()?;
    Ok(out)
}

impl Capture {
    /// Read the source dialect without rendering or replacing any declaration bytes.
    pub fn declaration(&self) -> Result<Declaration, String> {
        let text = |name: &str| {
            let bytes = self
                .files
                .get(&format!(".ecdev/declared/{name}"))
                .ok_or_else(|| format!("missing declaration {name}"))?;
            std::str::from_utf8(bytes).map_err(|e| e.to_string())
        };
        Ok(Declaration {
            repository: declare::parse_repository(text("repository.rs")?)
                .map_err(|_| "invalid source repository declaration")?,
            donors: declare::parse_donors(text("donors.rs")?)
                .map_err(|_| "invalid source donor declaration")?,
            migration: declare::parse_migration(text("migration.rs")?)
                .map_err(|_| "invalid source migration declaration")?,
            technologies: declare::parse_technologies(text("technologies.rs")?)
                .map_err(|_| "invalid source technology declaration")?,
        })
    }

    /// Validate transport integrity and native record decoding, never fresh proof validity.
    pub fn validate(&self) -> Result<(), String> {
        if self.files.len() > MAX_FILES {
            return Err("source capture file count exceeds budget".into());
        }
        let mut total = 0;
        for (path, bytes) in &self.files {
            total += bytes.len();
            if !allowed(path) || bytes.len() > MAX_FILE || total > MAX_TOTAL {
                return Err(format!("{path}: invalid source path or byte budget"));
            }
        }
        let d = self.declaration()?;
        if d.repository.id != "ecdev" || d.repository.origin.trim().is_empty() {
            return Err(
                "source capture needs the ECDEV source identity and declared origin".into(),
            );
        }
        let mut nodes = BTreeSet::new();
        let mut donors = BTreeSet::new();
        for node in &d.repository.nodes {
            if node.key.is_empty() || !nodes.insert(&node.key) {
                return Err(format!("empty or duplicate source node key: {}", node.key));
            }
        }
        for donor in &d.donors {
            if donor.key.is_empty() || !donors.insert(&donor.key) {
                return Err(format!(
                    "empty or duplicate source donor key: {}",
                    donor.key
                ));
            }
            let mut capabilities = BTreeSet::new();
            for capability in &donor.capabilities {
                if capability.key.is_empty() || !capabilities.insert(&capability.key) {
                    return Err(format!(
                        "empty or duplicate source capability key: {}/{}",
                        donor.key, capability.key
                    ));
                }
                if capability.absorbs.iter().any(|key| key.is_empty()) {
                    return Err(format!(
                        "empty absorbed source capability key: {}/{}",
                        donor.key, capability.key
                    ));
                }
            }
        }
        let (_, issues) = crate::graph::Graph::from_declaration(&d);
        if let Some(issue) = issues.first() {
            return Err(format!("source graph {}: {}", issue.code, issue.subject));
        }
        if !(self.observed_head.is_empty()
            || self.observed_head.len() == 40 && unhex(&self.observed_head).is_some())
        {
            return Err("invalid observed HEAD".into());
        }
        if !self.files.keys().any(|p| p.starts_with(".ecdev/history/")) {
            return Err(
                "source history is missing; denominator completeness is unavailable".into(),
            );
        }
        for (path, bytes) in &self.files {
            if path.starts_with(".ecdev/declared/") {
                continue;
            }
            if path.rsplit('/').next() != Some(evidence::address(bytes).as_str()) {
                return Err(format!("{path}: source content address mismatch"));
            }
            let result = if path.starts_with(".ecdev/history/") {
                Batch::decode(bytes).map(|_| ())
            } else if path.starts_with(".ecdev/knowledge/") {
                facts::decode(bytes).map(|_| ())
            } else if Decoder::tag_of(bytes) == Some(evidence::RETIRED_TAG) {
                evidence::decode_retired(bytes).map(|_| ())
            } else {
                evidence::Record::decode(bytes).map(|_| ())
            };
            result.map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(())
    }

    /// Same file entries as the engine's pinned source corpus: path, SHA-256 and lossless hex.
    pub fn render(&self) -> Result<String, String> {
        self.validate()?;
        let d = self.declaration()?;
        let (graph, _) = crate::graph::Graph::from_declaration(&d);
        Ok(Json::obj()
            .with("format", FORMAT)
            .with("binding", BINDING)
            .with("source_repository", d.repository.origin)
            .with("source_id", d.repository.id)
            .with("observed_head", &self.observed_head)
            .with("source_graph", graph.to_json())
            .with(
                "files",
                self.files
                    .iter()
                    .map(|(path, bytes)| {
                        Json::obj()
                            .with("path", path)
                            .with("sha256", &content_digest(bytes)[7..])
                            .with("hex", hex(bytes))
                    })
                    .collect::<Vec<_>>(),
            )
            .render())
    }

    /// Strict canonical transport reader. Rejects duplicate/extra fields and reordered paths
    /// by checking the canonical representation after native validation; writes nothing.
    pub fn parse(input: &str) -> Result<Capture, String> {
        if input.len() > MAX_TOTAL * 3 {
            return Err("source capture input exceeds budget".into());
        }
        let value = json::parse(input)?;
        if value.str("format") != Some(FORMAT) || value.str("binding") != Some(BINDING) {
            return Err("unsupported source capture format or binding".into());
        }
        let entries = match value.get("files") {
            Some(Value::Array(v)) if v.len() <= MAX_FILES => v,
            _ => return Err("invalid source file list".into()),
        };
        let mut out = Capture {
            observed_head: value
                .str("observed_head")
                .ok_or("missing observed HEAD")?
                .into(),
            files: BTreeMap::new(),
        };
        let mut total = 0;
        for f in entries {
            let path = f.str("path").ok_or("missing source path")?;
            let encoded = f.str("hex").ok_or("missing source bytes")?;
            if !allowed(path) || encoded.len() > MAX_FILE * 2 {
                return Err("invalid source file path or size".into());
            }
            let bytes = unhex(encoded).ok_or("invalid source hex")?;
            total += bytes.len();
            if total > MAX_TOTAL {
                return Err("source capture byte budget exceeded".into());
            }
            if f.str("sha256") != Some(&content_digest(&bytes)[7..]) {
                return Err(format!("{path}: source SHA-256 mismatch"));
            }
            if out.files.insert(path.into(), bytes).is_some() {
                return Err(format!("duplicate source path {path}"));
            }
        }
        if out.render()? != input {
            return Err("noncanonical source capture or inconsistent metadata".into());
        }
        Ok(out)
    }
}
