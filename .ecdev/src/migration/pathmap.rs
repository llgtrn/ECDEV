//! `legacy path → canonical node → canonical path`, planned in waves and applied one atomic wave
//! at a time. Applying a wave moves directories, rewrites Cargo workspace members and every
//! path dependency (relative paths recomputed from each manifest's new location), updates the
//! declarations (node paths and every declared repository path under a moved one: proof
//! locators, technology sources, donor source paths, shims) and marks the wave APPLIED. Node
//! identities are untouched: identity is never a path.

use crate::census::cargo::{join, members};
use crate::declare::{self, Declaration, Wave};
use crate::repository::files::Files;
use crate::repository::{role_of_path, ROLES};
use crate::schema::{is_physical, NodeKind, NodeLifecycle, WaveStatus};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One row of the path map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub legacy_path: String,
    pub node: String,
    pub node_id: String,
    pub canonical_path: String,
}

pub fn path_map(d: &Declaration) -> Vec<Mapping> {
    let mut v: Vec<Mapping> = d
        .repository
        .nodes
        .iter()
        .filter(|n| {
            is_physical(n.kind)
                && n.kind != NodeKind::Repository
                && n.lifecycle == NodeLifecycle::Active
        })
        .map(|n| Mapping {
            legacy_path: n.path.clone(),
            node: n.key.clone(),
            node_id: crate::graph::NodeId::of(crate::schema::NAMESPACE, &n.key).to_string(),
            canonical_path: n.canonical_path.clone(),
        })
        .collect();
    v.sort_by(|a, b| a.legacy_path.cmp(&b.legacy_path));
    v
}

/// Plans waves for every legacy-placed node not yet in a wave: one wave per current root,
/// ordered by the plane of their canonical roles (kernel first).
pub fn plan(d: &Declaration) -> Vec<Wave> {
    let planned: Vec<&str> = d
        .migration
        .waves
        .iter()
        .flat_map(|w| w.nodes.iter().map(String::as_str))
        .collect();
    let mut groups: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for m in path_map(d) {
        if m.legacy_path == m.canonical_path || planned.contains(&m.node.as_str()) {
            continue;
        }
        let root = m.legacy_path.split('/').next().unwrap_or("").to_string();
        let rank = role_of_path(&m.canonical_path)
            .and_then(|r| ROLES.iter().position(|s| s.role == r))
            .unwrap_or(ROLES.len());
        let e = groups.entry(root).or_insert((rank, Vec::new()));
        e.0 = e.0.min(rank);
        e.1.push(m.node);
    }
    let mut ordered: Vec<(usize, String, Vec<String>)> = groups
        .into_iter()
        .map(|(root, (rank, nodes))| (rank, root, nodes))
        .collect();
    ordered.sort();
    let base = d.migration.waves.len();
    ordered
        .into_iter()
        .enumerate()
        .map(|(i, (_, root, mut nodes))| {
            nodes.sort();
            Wave {
                key: format!("w{:02}-{}", base + i + 1, root.trim_start_matches('.')),
                status: WaveStatus::Planned,
                nodes,
            }
        })
        .collect()
}

/// The nodes away from their canonical path that are already assigned to a wave, grouped by
/// that wave (in declaration order): what is still to move when nothing is left to plan.
pub fn pending(d: &Declaration) -> Vec<(&Wave, Vec<Mapping>)> {
    let mut by_wave: Vec<(&Wave, Vec<Mapping>)> = Vec::new();
    for m in path_map(d) {
        if m.legacy_path == m.canonical_path {
            continue;
        }
        let Some(w) = d.migration.waves.iter().find(|w| w.nodes.contains(&m.node)) else {
            continue;
        };
        match by_wave.iter_mut().find(|(x, _)| x.key == w.key) {
            Some((_, v)) => v.push(m),
            None => by_wave.push((w, vec![m])),
        }
    }
    by_wave.sort_by_key(|(w, _)| d.migration.waves.iter().position(|x| x.key == w.key));
    by_wave
}

/// The relative path from directory `from` to `to` (both repository-relative).
pub fn relative(from: &str, to: &str) -> String {
    let f: Vec<&str> = from
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let t: Vec<&str> = to
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    let common = f.iter().zip(&t).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_string(), f.len() - common).collect();
    parts.extend(t[common..].iter().map(|s| s.to_string()));
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// Maps a path through a list of directory moves (applied in order).
pub fn map_path(moves: &[(String, String)], p: &str) -> String {
    let mut p = p.to_string();
    for (from, to) in moves {
        if p == *from {
            p = to.clone();
        } else if let Some(rest) = p.strip_prefix(&format!("{from}/")) {
            p = format!("{to}/{rest}");
        }
    }
    p
}

/// Rewrites every `path = "…"` value of a manifest that lived in `old_dir` and now lives in
/// `new_dir`, resolving each target through `moves`.
pub fn rewrite_paths(
    text: &str,
    old_dir: &str,
    new_dir: &str,
    moves: &[(String, String)],
) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i + 4 <= b.len() {
        let key_start = i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'\n' | b'{' | b',');
        if key_start && &b[i..i + 4] == b"path" {
            let mut j = i + 4;
            while j < b.len() && matches!(b[j], b' ' | b'\t') {
                j += 1;
            }
            if j < b.len() && b[j] == b'=' {
                j += 1;
                while j < b.len() && matches!(b[j], b' ' | b'\t') {
                    j += 1;
                }
                if j < b.len() && b[j] == b'"' {
                    let vs = j + 1;
                    if let Some(len) = text[vs..].find('"') {
                        let value = &text[vs..vs + len];
                        let target = map_path(moves, &join(old_dir, value));
                        let new_value = relative(new_dir, &target);
                        out.push_str(&text[last..vs]);
                        out.push_str(&new_value);
                        last = vs + len;
                        i = vs + len + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out.push_str(&text[last..]);
    out
}

/// Rewrites the string entries of `members`, `default-members` and `exclude` arrays.
pub fn rewrite_members(text: &str, moves: &[(String, String)]) -> String {
    let mut out = String::new();
    let mut in_array = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let opens = ["members", "default-members", "exclude"].iter().any(|k| {
            t.strip_prefix(k)
                .is_some_and(|r| r.trim_start().starts_with('='))
        });
        if opens {
            in_array = true;
        }
        if in_array {
            let mut l = String::new();
            let mut parts = line.split('"');
            l.push_str(parts.next().unwrap_or(""));
            for (k, part) in parts.enumerate() {
                l.push('"');
                if k % 2 == 0 {
                    l.push_str(&map_path(moves, part));
                } else {
                    l.push_str(part);
                }
            }
            out.push_str(&l);
            if line.contains(']') {
                in_array = false;
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Workspace references that do not resolve on disk: member directories without a manifest
/// and path dependencies whose target has none. Read from the working tree, not the index,
/// because it judges an in-flight wave.
pub fn unresolved_paths(root: &Path) -> Vec<String> {
    use crate::formats::{toml, Value};
    let mut out = Vec::new();
    let Some(ws) = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|t| toml::parse(&t).ok())
    else {
        return out;
    };
    let mut manifests: Vec<(String, Value)> = vec![(String::new(), ws.clone())];
    for m in ws
        .at("workspace")
        .map(|w| w.strings("members"))
        .unwrap_or_default()
    {
        let dirs: Vec<String> = match m.split_once('*') {
            Some((prefix, _)) => std::fs::read_dir(root.join(prefix.trim_end_matches('/')))
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter(|e| e.path().join("Cargo.toml").exists())
                        .map(|e| {
                            format!(
                                "{}/{}",
                                prefix.trim_end_matches('/'),
                                e.file_name().to_string_lossy()
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            None => vec![m.clone()],
        };
        for dir in dirs {
            match std::fs::read_to_string(root.join(&dir).join("Cargo.toml")) {
                Ok(t) => match toml::parse(&t) {
                    Ok(v) => manifests.push((dir, v)),
                    Err(e) => out.push(format!("{dir}/Cargo.toml: {e}")),
                },
                Err(_) => out.push(format!("member `{dir}` has no Cargo.toml")),
            }
        }
    }
    for (dir, v) in &manifests {
        let mut tables: Vec<&Value> = ["dependencies", "build-dependencies", "dev-dependencies"]
            .iter()
            .filter_map(|s| v.get(s))
            .collect();
        if let Some(Value::Table(t)) = v.get("target") {
            for x in t.values() {
                for s in ["dependencies", "build-dependencies", "dev-dependencies"] {
                    if let Some(d) = x.get(s) {
                        tables.push(d);
                    }
                }
            }
        }
        if let Some(d) = v.at("workspace.dependencies") {
            tables.push(d);
        }
        for t in tables {
            for (name, spec) in t.table().into_iter().flatten() {
                if let Some(p) = spec.str("path") {
                    let target = join(dir, p);
                    if !root.join(&target).join("Cargo.toml").exists() {
                        let at = if dir.is_empty() {
                            "Cargo.toml".to_string()
                        } else {
                            format!("{dir}/Cargo.toml")
                        };
                        out.push(format!("{at}: `{name}` -> `{target}` does not exist"));
                    }
                }
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct Applied {
    /// Every node of the wave that was away from its canonical path: (path before, canonical).
    /// A node nested inside another moved node moves with it.
    pub moves: Vec<(String, String)>,
    pub manifests: Vec<String>,
    /// Tracked files that still mention a moved path (not rewritten automatically).
    /// (file, what it references): moved paths it still names, and relative paths inside a
    /// moved tree whose resolution changed with the move.
    pub stale_references: Vec<(String, String)>,
    /// Technologies whose materialization locks were re-addressed to the moved destinations.
    pub locks: Vec<String>,
    /// Declared repository paths the wave rewrote.
    pub declared: Vec<Rewritten>,
}

/// One declared repository path a wave rewrote because it lay under a moved path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewritten {
    /// `donor`, `technology` or `shim`.
    pub owner: &'static str,
    pub key: String,
    /// `proof`, `spec`, `source_path`, `source`, `path` or `expiry`.
    pub field: &'static str,
    pub before: String,
    pub after: String,
}

/// A proof locator (`file::test`) through a list of directory moves: the file moves, the test
/// name does not.
pub fn map_locator(moves: &[(String, String)], locator: &str) -> String {
    match locator.rsplit_once("::") {
        Some((file, test)) => format!("{}::{test}", map_path(moves, file)),
        None => map_path(moves, locator),
    }
}

/// Whether a declared value is, as a whole, a repository path: one token of path characters,
/// optionally a locator (`file::test`) or an anchored document (`file#section`). Prose that
/// merely mentions a path (`one step for sim/replay/live; …`) is not one.
pub fn is_path_value(v: &str) -> bool {
    let file = v.split_once("::").map_or(v, |(f, _)| f);
    let file = file.split_once('#').map_or(file, |(f, _)| f);
    !file.is_empty()
        && file.bytes().all(path_char)
        && !v.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// A capability `spec` through a list of directory moves: rewritten only when the whole spec is
/// a repository path ([`is_path_value`]); a prose spec is never edited, even where it names a
/// moved path.
pub fn map_spec(moves: &[(String, String)], spec: &str) -> String {
    if !is_path_value(spec) {
        return spec.to_string();
    }
    match spec.split_once('#') {
        Some((file, anchor)) if !file.contains("::") => {
            format!("{}#{anchor}", map_path(moves, file))
        }
        _ => map_locator(moves, spec),
    }
}

/// Rewrites every declared repository path under a moved path: donors' source paths,
/// capability specs that are paths ([`map_spec`]; prose specs are never edited) and proof
/// locators, technologies' sources and proof and claim locators, and shim paths and legacy
/// expiries. Provenance keeps the paths of its time. Evidence is keyed by locator, so evidence
/// recorded under an old locator stays history: it proves nothing about the new one until the
/// proof is re-run.
pub fn rewrite_declared(d: &mut Declaration, moves: &[(String, String)]) -> Vec<Rewritten> {
    let mut out = Vec::new();
    let mut map = |owner: &'static str,
                   key: &str,
                   field: &'static str,
                   value: &mut String,
                   to: &dyn Fn(&str) -> String| {
        let after = to(value);
        if after != *value {
            out.push(Rewritten {
                owner,
                key: key.to_string(),
                field,
                before: std::mem::replace(value, after.clone()),
                after,
            });
        }
    };
    let path = |p: &str| map_path(moves, p);
    let locator = |p: &str| map_locator(moves, p);
    let spec = |p: &str| map_spec(moves, p);
    for dn in d.donors.iter_mut() {
        for p in dn.source_paths.iter_mut() {
            map("donor", &dn.key, "source_path", p, &path);
        }
        for c in dn.capabilities.iter_mut() {
            map("donor", &dn.key, "spec", &mut c.spec, &spec);
            for p in c.proofs.iter_mut() {
                map("donor", &dn.key, "proof", &mut p.locator, &locator);
            }
        }
    }
    for t in d.technologies.iter_mut() {
        for s in t.sources.iter_mut() {
            map("technology", &t.key, "source", s, &path);
        }
        for p in t
            .proofs
            .iter_mut()
            .chain(t.claims.iter_mut().flat_map(|c| c.evidence.iter_mut()))
        {
            map("technology", &t.key, "proof", &mut p.locator, &locator);
        }
    }
    for sh in d.migration.shims.iter_mut() {
        map("shim", &sh.key, "path", &mut sh.path, &path);
    }
    out
}

/// One filesystem change a wave made, kept so that a failure can undo it exactly.
enum Change {
    /// A file's bytes before the wave wrote it (`None`: it did not exist).
    File(PathBuf, Option<Vec<u8>>),
    /// The `.ecg` files of a state directory before the wave rewrote it.
    Dir(PathBuf, Vec<(std::ffi::OsString, Vec<u8>)>),
    Created(PathBuf),
    Removed(PathBuf),
    Renamed(PathBuf, PathBuf),
}

/// Everything a wave changed on disk, in order. `undo` reverts the changes in reverse order, so
/// each one is undone in exactly the state it produced.
#[derive(Default)]
struct Journal(Vec<Change>);

impl Journal {
    /// Creates `dir` and its missing ancestors.
    fn create_dirs(&mut self, dir: &Path) -> Result<(), String> {
        let mut missing = Vec::new();
        let mut at = Some(dir);
        while let Some(p) = at {
            if p.as_os_str().is_empty() || p.is_dir() {
                break;
            }
            if p.exists() {
                return Err(format!("`{}` is not a directory", p.display()));
            }
            missing.push(p.to_path_buf());
            at = p.parent();
        }
        for p in missing.into_iter().rev() {
            std::fs::create_dir(&p).map_err(|e| format!("create {}: {e}", p.display()))?;
            self.0.push(Change::Created(p));
        }
        Ok(())
    }
    fn rename(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if to.exists() {
            return Err("the destination already exists; refusing to overwrite".into());
        }
        std::fs::rename(from, to).map_err(|e| e.to_string())?;
        self.0
            .push(Change::Renamed(from.to_path_buf(), to.to_path_buf()));
        Ok(())
    }
    fn remove_empty_dir(&mut self, dir: &Path) -> bool {
        let removed = std::fs::remove_dir(dir).is_ok();
        if removed {
            self.0.push(Change::Removed(dir.to_path_buf()));
        }
        removed
    }
    /// Records a file's bytes before something rewrites it.
    fn keep(&mut self, p: &Path) {
        self.0
            .push(Change::File(p.to_path_buf(), std::fs::read(p).ok()));
    }
    fn write(&mut self, p: &Path, bytes: &str) -> Result<(), String> {
        self.keep(p);
        std::fs::write(p, bytes).map_err(|e| format!("write {}: {e}", p.display()))
    }
    /// Records the content-addressed files of a state directory before something rewrites them.
    fn keep_dir(&mut self, dir: &Path) {
        let files = std::fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".ecg"))
                    .filter_map(|e| Some((e.file_name(), std::fs::read(e.path()).ok()?)))
                    .collect()
            })
            .unwrap_or_default();
        self.0.push(Change::Dir(dir.to_path_buf(), files));
    }
    fn undo(self) {
        for c in self.0.into_iter().rev() {
            match c {
                Change::File(p, Some(b)) => {
                    let _ = std::fs::write(&p, b);
                }
                Change::File(p, None) => {
                    let _ = std::fs::remove_file(&p);
                }
                Change::Dir(dir, files) => {
                    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                        let name = e.file_name();
                        if name.to_string_lossy().ends_with(".ecg")
                            && !files.iter().any(|(n, _)| *n == name)
                        {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                    for (name, b) in files {
                        let _ = std::fs::write(dir.join(name), b);
                    }
                }
                Change::Created(p) => {
                    let _ = std::fs::remove_dir(&p);
                }
                Change::Removed(p) => {
                    let _ = std::fs::create_dir(&p);
                }
                Change::Renamed(from, to) => {
                    let _ = std::fs::rename(&to, &from);
                }
            }
        }
    }
}

fn manifest_path(dir: &str) -> String {
    if dir.is_empty() {
        "Cargo.toml".to_string()
    } else {
        format!("{dir}/Cargo.toml")
    }
}

/// A file's content if it is text (UTF-8, no NUL bytes, at most 4 MiB): binaries are not scanned.
fn read_text(p: &Path) -> Option<String> {
    let b = std::fs::read(p).ok()?;
    if b.len() > 4 << 20 || b.contains(&0) {
        return None;
    }
    String::from_utf8(b).ok()
}

fn path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/')
}

/// Whether the text before `at` ends with a relative prefix (`./`, `../`, `x/../`) or an
/// expansion (`${X}/`, `$(x)/`): what follows continues a path.
fn after_path_prefix(b: &[u8], at: usize) -> bool {
    let head = &b[..at];
    head.ends_with(b"./") || head.ends_with(b"}/") || head.ends_with(b")/") || after_variable(b, at)
}

/// Whether the text before `at` is `$NAME/`: a path continuing a shell variable.
fn after_variable(b: &[u8], at: usize) -> bool {
    if at < 2 || b[at - 1] != b'/' {
        return false;
    }
    let mut k = at - 1;
    while k > 0 && (b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'_') {
        k -= 1;
    }
    k < at - 1 && k > 0 && b[k - 1] == b'$'
}

/// Whether the value starting at `at` (just after its opening quote) is a `path = "…"` value.
fn cargo_path_value(b: &[u8], at: usize) -> bool {
    if at == 0 || b[at - 1] != b'"' {
        return false;
    }
    let mut k = at - 1;
    while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
        k -= 1;
    }
    if k == 0 || b[k - 1] != b'=' {
        return false;
    }
    k -= 1;
    while k > 0 && matches!(b[k - 1], b' ' | b'\t') {
        k -= 1;
    }
    k >= 4 && &b[k - 4..k] == b"path" && (k == 4 || !path_char(b[k - 5]))
}

/// Whether `text` names the directory `from` as a path: `from/…`, `./from`, `../from`,
/// `${ROOT}/from`, a Cargo `path = "from"`, or (for a nested `from`) the path itself. A bare
/// word such as `"control"` in a list is not a reference to the directory `control`.
pub fn mentions_path(text: &str, from: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while let Some(off) = text[i..].find(from) {
        let s = i + off;
        let e = s + from.len();
        i = s + from.chars().next().map_or(1, char::len_utf8);
        let prefixed = s > 0 && after_path_prefix(b, s);
        let starts = s == 0 || !path_char(b[s - 1]) || prefixed;
        let continues = e < b.len() && b[e] == b'/';
        let ends = e == b.len() || !path_char(b[e]) || continues;
        if starts && ends && (continues || prefixed || from.contains('/') || cargo_path_value(b, s))
        {
            return true;
        }
    }
    false
}

/// Resolves `rel` against directory `base`, keeping escapes above the root as `..`.
fn resolve(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." if parts.last().is_some_and(|l| *l != "..") => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Relative paths (`../…`) in `text`, the content of `file` (a path before the wave) inside a
/// moved tree, whose resolution changes with the move: (relative path, target before, target
/// after). Each is resolved against the file's directory and its crate root (the nearest
/// enclosing manifest), the bases a path in source, a test fixture or a build script is relative
/// to; when the target exists from one base only, that base decides. Cargo `path` values are
/// skipped in `manifest`s, which the wave already rewrote.
pub fn relative_escapes(
    files: &Files,
    moves: &[(String, String)],
    file: &str,
    text: &str,
    manifest: bool,
) -> Vec<(String, String, String)> {
    let dir = file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut crate_root = dir;
    while !crate_root.is_empty() && !files.paths.contains(&format!("{crate_root}/Cargo.toml")) {
        crate_root = crate_root.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    }
    let mut bases = vec![dir];
    if crate_root != dir && !crate_root.is_empty() {
        bases.push(crate_root);
    }
    let b = text.as_bytes();
    let mut out: Vec<(String, String, String)> = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find("../") {
        let s = i + off;
        let mut e = s;
        while e < b.len() && path_char(b[e]) {
            e += 1;
        }
        i = e.max(s + 3);
        // Only the start of a path token: not the inner `../` of `../../x` or `a/../x`.
        let starts = s == 0
            || !path_char(b[s - 1])
            || (b[s - 1] == b'/' && (s == 1 || !path_char(b[s - 2])))
            || after_variable(b, s);
        if !starts || (manifest && cargo_path_value(b, s)) {
            continue;
        }
        // A sentence's full stop is not part of the path (`../x.` but not `../..`).
        let rel = match text[s..e].strip_suffix('.') {
            Some(r) if !r.ends_with('.') && !r.ends_with('/') => r,
            _ => &text[s..e],
        };
        let judged: Vec<(String, String)> = bases
            .iter()
            .map(|base| {
                let was = resolve(base, rel);
                let now = resolve(&map_path(moves, base), rel);
                (was, now)
            })
            .collect();
        let existing: Vec<&(String, String)> =
            judged.iter().filter(|(was, _)| files.exists(was)).collect();
        let considered: Vec<&(String, String)> = if existing.is_empty() {
            judged.iter().collect()
        } else {
            existing
        };
        if let Some((was, now)) = considered
            .into_iter()
            .find(|(was, now)| map_path(moves, was) != *now)
        {
            let row = (rel.to_string(), was.clone(), now.clone());
            if !out.contains(&row) {
                out.push(row);
            }
        }
    }
    out
}

/// Applies one planned wave to the repository at `root`. A failure at any step leaves the
/// working tree and the declarations exactly as they were.
pub fn apply(
    root: &Path,
    files: &Files,
    d: &mut Declaration,
    wave_key: &str,
) -> Result<Applied, String> {
    let before = unresolved_paths(root);
    if !before.is_empty() {
        return Err(format!(
            "the workspace does not resolve before the wave: {}",
            before.join("; ")
        ));
    }
    let original = d.clone();
    let wi = d
        .migration
        .waves
        .iter()
        .position(|w| w.key == wave_key)
        .ok_or_else(|| format!("no wave `{wave_key}`"))?;
    if d.migration.waves[wi].status == WaveStatus::Applied {
        return Err(format!("wave `{wave_key}` is already applied"));
    }
    let mut moves: Vec<(String, String)> = Vec::new();
    for k in &d.migration.waves[wi].nodes {
        let n = d
            .node(k)
            .ok_or_else(|| format!("wave names undeclared node `{k}`"))?;
        if n.path != n.canonical_path {
            moves.push((n.path.clone(), n.canonical_path.clone()));
        }
    }
    // Outermost first: a node nested inside another moved node travels with it.
    moves.sort_by(|a, b| {
        a.0.matches('/')
            .count()
            .cmp(&b.0.matches('/').count())
            .then(a.0.cmp(&b.0))
    });
    for (from, _) in &moves {
        if !root.join(from).is_dir() {
            return Err(format!("`{from}` is not a directory"));
        }
    }
    // The physical moves, each from where the earlier ones left its node: a nested node whose
    // canonical path is its image under its parent's move needs none; any other moves again
    // after its parent, from its post-parent location.
    let mut sequenced: Vec<(String, String)> = Vec::new();
    for (from, to) in &moves {
        let now = map_path(&sequenced, from);
        if now == *to {
            continue;
        }
        if to.starts_with(&format!("{now}/")) {
            return Err(format!("cannot move `{from}` into itself (`{to}`)"));
        }
        if root.join(to).exists() {
            return Err(format!("`{to}` already exists; refusing to overwrite"));
        }
        sequenced.push((now, to.clone()));
    }

    let manifests_before: Vec<String> = {
        let mut v: Vec<String> = members(files)
            .into_iter()
            .filter(|m| m.dir != ".ecdev")
            .map(|m| m.dir)
            .collect();
        v.push(String::new());
        v.sort();
        v.dedup();
        v
    };
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    for dir in &manifests_before {
        if let Ok(t) = std::fs::read_to_string(root.join(manifest_path(dir))) {
            texts.insert(dir.clone(), t);
        }
    }

    let mut applied = Applied {
        moves: moves.clone(),
        ..Applied::default()
    };
    let mut journal = Journal::default();
    if let Err(e) = execute(root, d, wi, &sequenced, &texts, &mut journal, &mut applied) {
        journal.undo();
        *d = original;
        return Err(format!("wave `{wave_key}` rolled back: {e}"));
    }

    let rewritten: BTreeMap<String, &String> = texts
        .iter()
        .map(|(dir, text)| (manifest_path(dir), text))
        .collect();
    for f in &files.paths {
        if f.starts_with(".ecdev/") {
            continue;
        }
        let current = map_path(&sequenced, f);
        let manifest = rewritten.get(f);
        // A rewritten manifest is judged on its text before the wave; anything else is unchanged.
        let text = match manifest {
            Some(t) => Some((*t).clone()),
            None => read_text(&root.join(&current)),
        };
        let Some(t) = text else {
            continue;
        };
        // A decision record keeps the paths of its time (`audit::is_decision_record`).
        if manifest.is_none() && !crate::audit::is_decision_record(f) {
            for (from, _) in &moves {
                if mentions_path(&t, from) {
                    applied
                        .stale_references
                        .push((current.clone(), format!("`{from}`")));
                }
            }
        }
        if current != *f {
            for (rel, was, now) in relative_escapes(files, &sequenced, f, &t, manifest.is_some()) {
                applied.stale_references.push((
                    current.clone(),
                    format!("`{rel}` (resolved to `{was}`, now `{now}`)"),
                ));
            }
        }
    }
    applied.stale_references.dedup();
    Ok(applied)
}

/// The changes of a wave, every one through `journal` so that `apply` can undo them all.
fn execute(
    root: &Path,
    d: &mut Declaration,
    wi: usize,
    sequenced: &[(String, String)],
    texts: &BTreeMap<String, String>,
    journal: &mut Journal,
    applied: &mut Applied,
) -> Result<(), String> {
    for (from, to) in sequenced {
        let dest = root.join(to);
        if let Some(parent) = dest.parent() {
            journal.create_dirs(parent)?;
        }
        journal
            .rename(&root.join(from), &dest)
            .map_err(|e| format!("move {from} -> {to}: {e}"))?;
        // Remove emptied legacy parents.
        let mut parent = Path::new(from).parent();
        while let Some(p) = parent {
            if p.as_os_str().is_empty() || !journal.remove_empty_dir(&root.join(p)) {
                break;
            }
            parent = p.parent();
        }
    }

    for (old_dir, text) in texts {
        let new_dir = map_path(sequenced, old_dir);
        let mut t = rewrite_paths(text, old_dir, &new_dir, sequenced);
        if old_dir.is_empty() {
            t = rewrite_members(&t, sequenced);
        }
        if t != *text || new_dir != *old_dir {
            let rel = manifest_path(&new_dir);
            journal.write(&root.join(&rel), &t)?;
            applied.manifests.push(rel);
        }
    }

    for n in d.repository.nodes.iter_mut() {
        if is_physical(n.kind) && !n.path.is_empty() {
            n.path = map_path(sequenced, &n.path);
        }
    }
    applied.declared = rewrite_declared(d, sequenced);
    d.migration.waves[wi].status = WaveStatus::Applied;
    // The declarations' exact bytes, so an undo restores them as they were written.
    for f in [
        declare::REPOSITORY_FILE,
        declare::DONORS_FILE,
        declare::MIGRATION_FILE,
        declare::TECHNOLOGIES_FILE,
    ] {
        journal.keep(&declare::declared_dir(root).join(f));
    }
    declare::store(root, d).map_err(|e| format!("write declarations: {e}"))?;

    // Every wave leaves the workspace resolvable, or it is undone.
    let after = unresolved_paths(root);
    if !after.is_empty() {
        return Err(format!(
            "the workspace would not resolve: {}",
            after.join("; ")
        ));
    }

    // Materialized copies moved with their node: their locks follow, so they still verify.
    journal.keep_dir(&root.join(crate::technology::MATERIALIZED_DIR));
    applied.locks = crate::technology::relocate_locks(root, |p| map_path(sequenced, p))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wave_that_breaks_resolution_is_rolled_back() {
        use crate::declare::*;
        use crate::schema::*;
        let root = std::env::temp_dir().join(format!("ecdev-gov-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w(
            "Cargo.toml",
            "[workspace]\nmembers = [\"core\", \"storage\", \"tools/*\"]\n",
        );
        w("core/Cargo.toml", "[package]\nname = \"core\"\n");
        w(
            "storage/Cargo.toml",
            "[package]\nname = \"storage\"\n[dependencies]\ncore = { path = \"../core\" }\n",
        );
        // A crate a concurrent branch just created under a glob member: on disk, not yet indexed.
        w(
            "tools/fresh/Cargo.toml",
            "[package]\nname = \"fresh\"\n[dependencies]\nstorage = { path = \"../../storage\" }\n",
        );
        let node = |k: &str, kind, p: &str, c: &str| Node::new(k, kind, p, c);
        let mut d = Declaration {
            technologies: vec![],
            repository: Repository {
                id: "ecdev".into(),
                name: "t".into(),
                origin: "t".into(),
                nodes: vec![
                    node("core", NodeKind::Kernel, "core", "core"),
                    node(
                        "storage",
                        NodeKind::Substrate,
                        "storage",
                        "substrate/storage",
                    ),
                ],
                edges: vec![],
            },
            donors: vec![],
            migration: Migration {
                waves: vec![Wave {
                    key: "w1".into(),
                    status: WaveStatus::Planned,
                    nodes: vec!["storage".into()],
                }],
                shims: vec![],
            },
        };
        store(&root, &d).unwrap();
        // Laid out by hand (not the canonical rendering): a rollback restores these exact bytes.
        let repo_file = declared_dir(&root).join(REPOSITORY_FILE);
        let hand = format!(
            "// Kept by hand.\n{}",
            std::fs::read_to_string(&repo_file).unwrap()
        );
        std::fs::write(&repo_file, &hand).unwrap();
        let indexed = Files::from_paths(
            &root,
            ["Cargo.toml", "core/Cargo.toml", "storage/Cargo.toml"].map(String::from),
        );
        assert!(unresolved_paths(&root).is_empty());
        let err = apply(&root, &indexed, &mut d, "w1").unwrap_err();
        assert!(
            err.contains("rolled back") && err.contains("tools/fresh"),
            "{err}"
        );
        assert!(
            root.join("storage/Cargo.toml").exists()
                && !root.join("substrate").join("storage").exists()
        );
        assert_eq!(
            load(&root).unwrap().migration.waves[0].status,
            WaveStatus::Planned
        );
        assert_eq!(std::fs::read_to_string(&repo_file).unwrap(), hand);
        assert!(
            unresolved_paths(&root).is_empty(),
            "the tree is exactly as before"
        );
        // With the new crate indexed, the same wave applies and resolves.
        let all = Files::scan(&root).unwrap();
        let mut d = load(&root).unwrap();
        apply(&root, &all, &mut d, "w1").unwrap();
        assert!(unresolved_paths(&root).is_empty());
        assert!(std::fs::read_to_string(root.join("tools/fresh/Cargo.toml"))
            .unwrap()
            .contains("\"../../substrate/storage\""));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every file and directory under `root` with its bytes (directories map to `None`).
    fn tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(root: &Path, rel: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            let mut entries: Vec<_> = std::fs::read_dir(root.join(rel))
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let p = rel.join(e.file_name());
                let key = p.to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    out.insert(key, None);
                    walk(root, &p, out);
                } else {
                    out.insert(key, Some(std::fs::read(root.join(&p)).unwrap()));
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, Path::new(""), &mut out);
        out
    }

    fn repository(nodes: Vec<crate::declare::Node>, wave: &[&str]) -> Declaration {
        Declaration {
            technologies: vec![],
            repository: crate::declare::Repository {
                id: "ecdev".into(),
                name: "t".into(),
                origin: "t".into(),
                nodes,
                edges: vec![],
            },
            donors: vec![],
            migration: crate::declare::Migration {
                waves: vec![Wave {
                    key: "w1".into(),
                    status: WaveStatus::Planned,
                    nodes: wave.iter().map(|s| s.to_string()).collect(),
                }],
                shims: vec![],
            },
        }
    }

    #[test]
    fn nested_nodes_of_one_wave_move_with_their_parent() {
        let root = std::env::temp_dir().join(format!("ecdev-gov-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w(
            "Cargo.toml",
            "[workspace]\nmembers = [\"core\", \"world\", \"world/calendar\", \"world/clock\"]\n",
        );
        w("core/Cargo.toml", "[package]\nname = \"core\"\n");
        w(
            "world/Cargo.toml",
            "[package]\nname = \"world\"\n[dependencies]\ncalendar = { path = \"calendar\" }\nclock = { path = \"clock\" }\n",
        );
        w("world/src/lib.rs", "pub fn w() {}\n");
        w(
            "world/calendar/Cargo.toml",
            "[package]\nname = \"calendar\"\n[dependencies]\ncore = { path = \"../../core\" }\n",
        );
        w(
            "world/clock/Cargo.toml",
            "[package]\nname = \"clock\"\n[dependencies]\ncalendar = { path = \"../calendar\" }\n",
        );
        let node = |k: &str, p: &str, c: &str| crate::declare::Node::new(k, NodeKind::Domain, p, c);
        // `world/calendar` lands at its parent-relative image; `world/clock` does not.
        let mut d = repository(
            vec![
                crate::declare::Node::new("core", NodeKind::Kernel, "core", "core"),
                node("world", "world", "domain/world"),
                node("world.calendar", "world/calendar", "domain/world/calendar"),
                node("world.clock", "world/clock", "domain/clock"),
            ],
            &["world", "world.calendar", "world.clock"],
        );
        declare::store(&root, &d).unwrap();
        let files = Files::scan(&root).unwrap();
        let r = apply(&root, &files, &mut d, "w1").unwrap();
        assert_eq!(
            r.moves,
            vec![
                ("world".to_string(), "domain/world".to_string()),
                (
                    "world/calendar".to_string(),
                    "domain/world/calendar".to_string()
                ),
                ("world/clock".to_string(), "domain/clock".to_string()),
            ]
        );
        assert!(!root.join("world").exists());
        assert!(root.join("domain/world/src/lib.rs").exists());
        assert!(root.join("domain/world/calendar/Cargo.toml").exists());
        assert!(root.join("domain/clock/Cargo.toml").exists());
        assert!(!root.join("domain/world/clock").exists());
        assert!(
            unresolved_paths(&root).is_empty(),
            "{:?}",
            unresolved_paths(&root)
        );
        let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
        assert!(read("domain/world/Cargo.toml").contains("clock = { path = \"../clock\" }"));
        assert!(read("domain/world/calendar/Cargo.toml").contains("\"../../../core\""));
        assert!(read("domain/clock/Cargo.toml").contains("\"../world/calendar\""));
        let after = declare::load(&root).unwrap();
        for n in &after.repository.nodes {
            assert_eq!(n.path, n.canonical_path, "{}", n.key);
        }
        assert_eq!(after.migration.waves[0].status, WaveStatus::Applied);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_move_that_fails_mid_wave_restores_the_exact_prior_tree() {
        let root = std::env::temp_dir().join(format!("ecdev-gov-midfail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w(
            "Cargo.toml",
            "[workspace]\nmembers = [\"aa\", \"zz/bb\", \"legacy/cc\"]\n",
        );
        w("aa/Cargo.toml", "[package]\nname = \"aa\"\n");
        w(
            "zz/bb/Cargo.toml",
            "[package]\nname = \"bb\"\n[dependencies]\naa = { path = \"../../aa\" }\n",
        );
        w("legacy/cc/Cargo.toml", "[package]\nname = \"cc\"\n");
        // `blocked` is a file: `bb` cannot be moved beneath it, after `aa` and `legacy/cc` moved.
        w("blocked", "not a directory\n");
        let node =
            |k: &str, p: &str, c: &str| crate::declare::Node::new(k, NodeKind::Substrate, p, c);
        let mut d = repository(
            vec![
                node("aa", "aa", "substrate/aa"),
                node("bb", "zz/bb", "blocked/bb"),
                node("cc", "legacy/cc", "substrate/cc"),
            ],
            &["aa", "bb", "cc"],
        );
        declare::store(&root, &d).unwrap();
        let original = d.clone();
        let before = tree(&root);
        let files = Files::scan(&root).unwrap();
        let err = apply(&root, &files, &mut d, "w1").unwrap_err();
        assert!(
            err.contains("rolled back") && err.contains("blocked"),
            "{err}"
        );
        assert_eq!(
            tree(&root),
            before,
            "every move, directory and file is restored"
        );
        assert_eq!(d, original);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_path_shaped_mentions_are_references() {
        for yes in [
            "see control/README.md",
            "cd ./control && make",
            "include!(\"../control/x.rs\")",
            "dep = { path = \"control\" }",
            "dep = {path=\"control\"}",
            "${ROOT}/control",
            "$ROOT/control",
            "`control/`",
            "control/",
        ] {
            assert!(mentions_path(yes, "control"), "{yes}");
        }
        for no in [
            "tags = [\"control\", \"io\"]",
            "the control plane",
            "src/control/mod.rs",
            "https://example.com/control/x",
            "control_plane/",
            "control.rs",
            "use crate::control::x;",
            "name = \"control\"",
        ] {
            assert!(!mentions_path(no, "control"), "{no}");
        }
        assert!(mentions_path("\"legacy/control\"", "legacy/control"));
        assert!(!mentions_path("\"mylegacy/control\"", "legacy/control"));
    }

    #[test]
    fn relative_escapes_that_change_with_depth_are_found() {
        let root = Path::new("/nonexistent");
        let files = Files::from_paths(
            root,
            [
                "storage/Cargo.toml",
                "storage/src/lib.rs",
                "storage/data.txt",
                "storage/tests/t.rs",
                "storage/fixtures/a.txt",
                "fixtures/b.txt",
                "core/Cargo.toml",
            ]
            .map(String::from),
        );
        let moves = vec![("storage".to_string(), "substrate/storage".to_string())];
        let lib = "const T: &str = concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../target\");\nconst D: &str = include_str!(\"../data.txt\");\n";
        let found = relative_escapes(&files, &moves, "storage/src/lib.rs", lib, false);
        assert_eq!(
            found,
            vec![(
                "../target".to_string(),
                "target".to_string(),
                "substrate/target".to_string()
            )]
        );
        // From a test, cargo runs in the crate root: `../fixtures` left the crate.
        let t = "let b = \"../fixtures/b.txt\"; let a = \"../fixtures/a.txt\"; // see ../../core.";
        let found = relative_escapes(&files, &moves, "storage/tests/t.rs", t, false);
        assert_eq!(
            found
                .iter()
                .map(|(r, w, n)| format!("{r} {w} {n}"))
                .collect::<Vec<_>>(),
            vec![
                "../fixtures/b.txt fixtures/b.txt substrate/fixtures/b.txt",
                "../../core core substrate/core",
            ]
        );
        // A manifest's `path` values were rewritten by the wave; its other relative values were not.
        let m =
            "[package]\nreadme = \"../README.md\"\n[dependencies]\ncore = { path = \"../core\" }\n";
        let found = relative_escapes(&files, &moves, "storage/Cargo.toml", m, true);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, "../README.md");
        // Within the moved tree nothing changes.
        assert!(relative_escapes(
            &files,
            &moves,
            "storage/src/lib.rs",
            "\"../data.txt\"",
            false
        )
        .is_empty());
    }

    #[test]
    fn a_wave_reports_references_in_every_tracked_text_file() {
        let root = std::env::temp_dir().join(format!("ecdev-gov-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, t: &[u8]| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w(
            "Cargo.toml",
            b"[workspace]\nmembers = [\"core\", \"control\"]\n",
        );
        w("core/Cargo.toml", b"[package]\nname = \"core\"\n");
        w(
            "control/Cargo.toml",
            b"[package]\nname = \"control\"\nreadme = \"../README.md\"\n[dependencies]\ncore = { path = \"../core\" }\n",
        );
        w(
            "control/src/lib.rs",
            b"pub const T: &str = concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../target\");\n",
        );
        w("README.md", b"Build control/ first.\n");
        w("web/app.ts", b"import x from '../control/pkg';\n");
        w("native/x.h", b"#include \"control/api.h\"\n");
        w("tools/gen.py", b"ROOT = './control'\n");
        w("data/tags.json", b"{\"tags\": [\"control\", \"core\"]}\n");
        w(".gitignore", b"control/target\n");
        w("assets/logo.bin", b"control/\0\x01\x02");
        w(
            "docs/decisions/0003-control-plane.md",
            b"Put `control/` at the root.\n",
        );
        w("docs/decisions/README.md", b"- 0003: see control/\n");
        let node = |k: &str, kind, p: &str, c: &str| crate::declare::Node::new(k, kind, p, c);
        let mut d = Declaration {
            technologies: vec![],
            repository: crate::declare::Repository {
                id: "ecdev".into(),
                name: "t".into(),
                origin: "t".into(),
                nodes: vec![
                    node("core", NodeKind::Kernel, "core", "core"),
                    node(
                        "control",
                        NodeKind::Substrate,
                        "control",
                        "substrate/control",
                    ),
                ],
                edges: vec![],
            },
            donors: vec![],
            migration: crate::declare::Migration {
                waves: vec![Wave {
                    key: "w1".into(),
                    status: WaveStatus::Planned,
                    nodes: vec!["control".into()],
                }],
                shims: vec![],
            },
        };
        declare::store(&root, &d).unwrap();
        let files = Files::scan(&root).unwrap();
        let r = apply(&root, &files, &mut d, "w1").unwrap();
        let mut got: Vec<String> = r
            .stale_references
            .iter()
            .map(|(f, p)| format!("{f}: {p}"))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                ".gitignore: `control`",
                "README.md: `control`",
                "docs/decisions/README.md: `control`",
                "native/x.h: `control`",
                "substrate/control/Cargo.toml: `../README.md` (resolved to `README.md`, now `substrate/README.md`)",
                "substrate/control/src/lib.rs: `../target` (resolved to `target`, now `substrate/target`)",
                "tools/gen.py: `control`",
                "web/app.ts: `control`",
            ],
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_wave_leaves_unchanged_declarations_byte_identical() {
        use crate::declare::*;
        let root = std::env::temp_dir().join(format!("ecdev-gov-noise-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, t: &str| {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), t).unwrap();
        };
        w(
            "Cargo.toml",
            "[workspace]\nmembers = [\"core\", \"control\"]\n",
        );
        w("core/Cargo.toml", "[package]\nname = \"core\"\n");
        w("control/Cargo.toml", "[package]\nname = \"control\"\n");
        let node = |k: &str, kind, p: &str, c: &str| Node::new(k, kind, p, c);
        let mut d = Declaration {
            technologies: vec![],
            repository: Repository {
                id: "ecdev".into(),
                name: "t".into(),
                origin: "t".into(),
                nodes: vec![
                    node("core", NodeKind::Kernel, "core", "core"),
                    node(
                        "control",
                        NodeKind::Substrate,
                        "control",
                        "substrate/control",
                    ),
                ],
                edges: vec![],
            },
            donors: vec![],
            migration: Migration {
                waves: vec![Wave {
                    key: "w1".into(),
                    status: WaveStatus::Planned,
                    nodes: vec!["control".into()],
                }],
                shims: vec![],
            },
        };
        store(&root, &d).unwrap();
        // Written by a human or agent in a compact layout the canonical renderer would not choose:
        // one-line `Proof`s and an invariant list wider than the canonical width.
        let technologies = "// ECDEV governance declaration.\n&[\n    Technology {\n        key: \"codec.base64\",\n        name: \"RFC 4648 base64\",\n        kind: TechnologyKind::Codec,\n        claimed: TechnologyLifecycle::Canonical,\n        purpose: \"text encoding of bytes\",\n        implements: &[\"serialization.base64\"],\n        node: \"core\",\n        sources: &[\"core/src/base64.rs\"],\n        invariants: &[\"RFC 4648 vectors, standard and URL-safe alphabets\", \"every byte string round-trips\", \"decoding is strict\"],\n        proofs: &[\n            Proof { kind: ProofKind::Regression, locator: \"core/src/base64.rs::rfc4648_vectors\" },\n        ],\n        lineage: &[],\n        relations: &[],\n        claims: &[],\n    },\n]\n";
        let donors = "// Donors (none yet).\n&[]\n";
        let dir = declared_dir(&root);
        std::fs::write(dir.join(TECHNOLOGIES_FILE), technologies).unwrap();
        std::fs::write(dir.join(DONORS_FILE), donors).unwrap();
        assert_ne!(
            render_technologies(&parse_technologies(technologies).ok().unwrap()),
            technologies,
            "the fixture is not the canonical rendering"
        );
        let mut d2 = load(&root).unwrap();
        assert_eq!(d2.technologies.len(), 1);
        d.technologies = d2.technologies.clone();
        assert_eq!(d2, d);

        let files = Files::scan(&root).unwrap();
        apply(&root, &files, &mut d2, "w1").unwrap();
        assert!(root.join("substrate/control/Cargo.toml").exists());
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        assert_eq!(
            read(TECHNOLOGIES_FILE),
            technologies,
            "unchanged: untouched"
        );
        assert_eq!(read(DONORS_FILE), donors, "unchanged: untouched");
        // What the wave changed is written (canonically).
        let after = load(&root).unwrap();
        assert_eq!(after.node("control").unwrap().path, "substrate/control");
        assert_eq!(after.migration.waves[0].status, WaveStatus::Applied);
        assert_eq!(read(REPOSITORY_FILE), render_repository(&after.repository));

        // A changed technology is rewritten canonically.
        let mut d3 = after.clone();
        d3.technologies[0].purpose = "bytes as text".into();
        store(&root, &d3).unwrap();
        assert_eq!(
            read(TECHNOLOGIES_FILE),
            render_technologies(&d3.technologies)
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_prose_spec_is_never_rewritten() {
        let mut d = repository(vec![], &[]);
        let cap = |key: &str, spec: &str| crate::declare::Capability {
            key: key.into(),
            required: true,
            spec: spec.into(),
            replacement: None,
            maps_to: None,
            relevance: Default::default(),
            proofs: vec![],
        };
        d.donors = vec![crate::declare::Donor {
            key: "d".into(),
            name: "d".into(),
            origin: "o".into(),
            license: "MIT".into(),
            claimed: crate::schema::DonorState::Discovered,
            exception: None,
            packages: vec![],
            source_paths: vec!["simulation/vendor".into()],
            capabilities: vec![
                cap(
                    "step",
                    "One engine step for simulation/replay/live; record + replay with digests",
                ),
                cap("modes", "simulation/replay/live modes are deterministic"),
                cap("file", "simulation/tests/parity.rs"),
                cap("test", "simulation/tests/parity.rs::step_matches"),
                cap("section", "simulation/README.md#replay"),
                cap("dir", "simulation"),
                cap("elsewhere", "world/spec.md"),
            ],
            cutover: None,
            provenance: vec!["simulation/old".into()],
        }];
        let before = d.clone();
        let moves = vec![("simulation".to_string(), "domain/simulation".to_string())];
        let out = rewrite_declared(&mut d, &moves);
        let specs: Vec<&str> = d.donors[0]
            .capabilities
            .iter()
            .map(|c| c.spec.as_str())
            .collect();
        assert_eq!(
            specs,
            vec![
                "One engine step for simulation/replay/live; record + replay with digests",
                "simulation/replay/live modes are deterministic",
                "domain/simulation/tests/parity.rs",
                "domain/simulation/tests/parity.rs::step_matches",
                "domain/simulation/README.md#replay",
                "domain/simulation",
                "world/spec.md",
            ]
        );
        assert_eq!(d.donors[0].source_paths, vec!["domain/simulation/vendor"]);
        assert_eq!(d.donors[0].provenance, before.donors[0].provenance);
        assert_eq!(
            out.iter().filter(|r| r.field == "spec").count(),
            4,
            "{out:?}"
        );
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative("storage", "core"), "../core");
        assert_eq!(relative("substrate/storage", "core"), "../../core");
        assert_eq!(relative("", "substrate/storage"), "substrate/storage");
        assert_eq!(relative("apps/cli", "apps/cli/sub"), "sub");
    }

    #[test]
    fn manifests_are_rewritten_through_moves() {
        let moves = vec![
            ("storage".to_string(), "substrate/storage".to_string()),
            ("world".to_string(), "substrate/world".to_string()),
        ];
        let m = "[dependencies]\ncore = { path = \"../core\" }\nworld = {path=\"../world\", version = \"0.1\"}\n";
        let out = rewrite_paths(m, "storage", "substrate/storage", &moves);
        assert!(out.contains("path = \"../../core\""), "{out}");
        assert!(out.contains("path=\"../world\""), "{out}");
        let root = "[workspace]\nmembers = [\n  \"core\",\n  \"storage\",\n  \"world\", # c\n]\n[workspace.dependencies]\nstorage = { path = \"storage\" }\n";
        let r = rewrite_members(&rewrite_paths(root, "", "", &moves), &moves);
        assert!(
            r.contains("\"substrate/storage\",\n  \"substrate/world\", # c"),
            "{r}"
        );
        assert!(
            r.contains("storage = { path = \"substrate/storage\" }"),
            "{r}"
        );
        assert!(r.contains("\"core\""));
    }
}
