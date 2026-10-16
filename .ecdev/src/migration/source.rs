//! Read-only transport of source governance bytes. This is a capture, never enrollment,
//! a canonical declaration, proof execution, or an attestation of repository-history completeness.

use crate::compact::{codec::Decoder, facts, history::Batch};
use crate::declare::{self, Declaration};
use crate::digest::{content_digest, hex, unhex};
use crate::evidence;
use crate::formats::{json, json::Json, Value};
use crate::graph::Graph;
use crate::repository::files::head_commit;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

pub const FORMAT: &str = "ecdev-source-capture-v1";
const BINDING: &str = "CAPTURED_BYTES; HEAD_OBSERVED_ONLY; NOT_ENROLLED; NOT_REVERIFIED";
const DIRS: &[&str] = &["declared", "history", "evidence", "knowledge"];
const MAX_FILE: usize = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 16_384;
const MAX_EXPORT: usize = MAX_TOTAL * 3;

/// Local object reads never run hooks, filters or source programs. Replace refs must not
/// substitute different objects for the immutable commit selected at the start of export.
fn git_bytes(root: &Path, args: &[&str], budget: usize) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("source object read: {e}"))?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .ok_or("missing Git output")?
        .take((budget + 1) as u64)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() > budget {
        let _ = child.kill();
        let _ = child.wait();
        return Err("source object read exceeds budget or failed".into());
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "source object read `git {}` failed: {status}",
            args.join(" ")
        ));
    }
    Ok(bytes)
}

/// Export the selected HEAD's object-tree bytes, ignoring the index and working tree.
/// The portable v1 binding remains observation-only: an offline reader cannot establish
/// Git object membership or repository-history completeness from this envelope alone.
pub fn capture_committed(root: &Path) -> Result<Capture, String> {
    let head = git_bytes(root, &["rev-parse", "--verify", "HEAD^{commit}"], 128)?;
    let head = std::str::from_utf8(&head)
        .map_err(|e| e.to_string())?
        .trim();
    if head.len() != 40 || unhex(head).is_none() {
        return Err("source HEAD must resolve to a full commit SHA".into());
    }
    let controls = git_bytes(
        root,
        &["ls-tree", "-z", &format!("{head}:.ecdev")],
        1024 * 1024,
    )?;
    for dir in DIRS {
        let found = controls
            .split(|b| *b == 0)
            .filter_map(|row| std::str::from_utf8(row).ok())
            .any(|row| {
                row.starts_with("040000 tree ")
                    && row.split_once('\t').is_some_and(|(_, path)| path == *dir)
            });
        if !found {
            return Err(format!(
                "committed .ecdev/{dir}: source directory missing or not a tree"
            ));
        }
    }
    let entries = git_bytes(
        root,
        &[
            "ls-tree",
            "-r",
            "-z",
            "-l",
            "--full-tree",
            head,
            "--",
            ".ecdev/declared",
            ".ecdev/history",
            ".ecdev/evidence",
            ".ecdev/knowledge",
        ],
        4 * 1024 * 1024,
    )?;
    let mut out = Capture {
        observed_head: head.into(),
        source_schema: crate::protocol::schema_identity(),
        files: BTreeMap::new(),
    };
    let mut total = 0usize;
    for row in entries.split(|b| *b == 0).filter(|row| !row.is_empty()) {
        let row = std::str::from_utf8(row).map_err(|_| "non UTF-8 committed source path")?;
        let (meta, path) = row.split_once('\t').ok_or("invalid source tree entry")?;
        let fields: Vec<_> = meta.split_whitespace().collect();
        if fields.len() != 4
            || !matches!(fields[0], "100644" | "100755")
            || fields[1] != "blob"
            || !allowed(path)
            || fields[2].len() != 40
            || unhex(fields[2]).is_none()
        {
            return Err(format!("{path}: unsupported committed source entry"));
        }
        let len: usize = fields[3]
            .parse()
            .map_err(|_| "invalid source blob length")?;
        total = total
            .checked_add(len)
            .ok_or("source capture byte budget exceeded")?;
        if len > MAX_FILE || total > MAX_TOTAL || out.files.len() >= MAX_FILES {
            return Err("committed source exceeds capture budget".into());
        }
        let bytes = git_bytes(root, &["cat-file", "blob", fields[2]], len)?;
        if bytes.len() != len || out.files.insert(path.into(), bytes).is_some() {
            return Err(format!(
                "{path}: inconsistent or duplicate source tree entry"
            ));
        }
    }
    out.validate()?;
    Ok(out)
}

/// Portable read-only caller: bounded regular-file input, strict existing v1 transport.
pub fn read_export(path: &Path) -> Result<Capture, String> {
    read_export_bound(path, None)
}

/// Bind exact envelope bytes to a registry digest obtained independently by the caller.
/// Successful integrity validation still does not establish Git membership or admission.
pub fn read_export_bound(path: &Path, expected_sha256: Option<&str>) -> Result<Capture, String> {
    if expected_sha256.is_some_and(|hash| hash.len() != 64 || unhex(hash).is_none()) {
        return Err("expected source export SHA-256 must be 64 hex digits".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_EXPORT as u64 {
        return Err("source export must be a regular file within the reader budget".into());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.take((MAX_EXPORT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_EXPORT {
        return Err("source export exceeds reader budget".into());
    }
    if expected_sha256.is_some_and(|hash| !hash.eq_ignore_ascii_case(&content_digest(&bytes)[7..]))
    {
        return Err("source export does not match caller-bound SHA-256".into());
    }
    Capture::parse(std::str::from_utf8(&bytes).map_err(|e| format!("source export UTF-8: {e}"))?)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// The observed HEAD, not a claim that these working-tree bytes belong to that commit.
    pub observed_head: String,
    /// Recorded producer schema claim, not the reader's authority. Command-only schema
    /// changes must not rewrite or reject an older portable capture's graph metadata.
    pub source_schema: String,
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
        source_schema: crate::protocol::schema_identity(),
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
    /// Recompute source identities and graph content while retaining the recorded schema
    /// digest as unverified metadata. No source hash selects a reader or grants authority.
    pub fn source_graph(&self) -> Result<Json, String> {
        let mut graph = Graph::from_declaration(&self.declaration()?).0.to_json();
        if let Json::Object(fields) = &mut graph {
            for (key, value) in fields {
                if key == "schema" {
                    *value = Json::from(&self.source_schema);
                }
            }
        }
        Ok(graph)
    }
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
        if !self
            .source_schema
            .strip_prefix("sha256:")
            .is_some_and(|hash| hash.len() == 64 && unhex(hash).is_some())
        {
            return Err("invalid recorded source schema digest".into());
        }
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
        Ok(Json::obj()
            .with("format", FORMAT)
            .with("binding", BINDING)
            .with("source_repository", d.repository.origin)
            .with("source_id", d.repository.id)
            .with("observed_head", &self.observed_head)
            .with("source_graph", self.source_graph()?)
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
        if input.len() > MAX_EXPORT {
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
            source_schema: value
                .at("source_graph.schema")
                .and_then(Value::as_str)
                .ok_or("missing recorded source schema")?
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
