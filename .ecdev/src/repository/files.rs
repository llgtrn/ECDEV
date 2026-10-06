//! The file universe of a repository: the paths Git tracks (read natively from `.git/index`),
//! or, where there is no Git directory (fixtures), every file below the root. Using the index
//! makes every census a function of committed content, identical on every checkout.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Files {
    pub root: PathBuf,
    /// Repository-relative paths with `/` separators, sorted.
    pub paths: BTreeSet<String>,
    /// Gitlinks (submodule entries), which are foreign code by construction.
    pub gitlinks: BTreeSet<String>,
    pub from_index: bool,
}

impl Files {
    pub fn scan(root: &Path) -> std::io::Result<Files> {
        if let Some(index) = git_index_path(root) {
            if let Ok(bytes) = std::fs::read(&index) {
                if let Ok((paths, gitlinks)) = parse_index(&bytes) {
                    return Ok(Files {
                        root: root.to_path_buf(),
                        paths,
                        gitlinks,
                        from_index: true,
                    });
                }
            }
        }
        let mut paths = BTreeSet::new();
        walk(root, root, &mut paths)?;
        Ok(Files {
            root: root.to_path_buf(),
            paths,
            gitlinks: BTreeSet::new(),
            from_index: false,
        })
    }

    /// Builds a universe from an explicit path list (tests, dry runs).
    pub fn from_paths(root: &Path, paths: impl IntoIterator<Item = String>) -> Files {
        Files {
            root: root.to_path_buf(),
            paths: paths.into_iter().collect(),
            gitlinks: BTreeSet::new(),
            from_index: false,
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        let path = path.trim_end_matches('/');
        self.paths.contains(path)
            || self.under(path).next().is_some()
            || self.gitlinks.contains(path)
    }

    /// Tracked paths under a directory.
    pub fn under<'a>(&'a self, dir: &str) -> impl Iterator<Item = &'a String> + 'a {
        let prefix = if dir.is_empty() || dir == "." {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        self.paths
            .range(prefix.clone()..)
            .take_while(move |p| p.starts_with(&prefix))
    }

    pub fn with_suffix<'a>(&'a self, suffix: &'a str) -> impl Iterator<Item = &'a String> + 'a {
        self.paths.iter().filter(move |p| p.ends_with(suffix))
    }

    pub fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }

    pub fn read_bytes(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.root.join(path)).ok()
    }

    /// Top-level entries (first path segments), files and directories alike.
    pub fn root_entries(&self) -> BTreeSet<String> {
        self.paths
            .iter()
            .chain(self.gitlinks.iter())
            .map(|p| p.split('/').next().unwrap_or(p).to_string())
            .collect()
    }

    pub fn is_dir(&self, entry: &str) -> bool {
        self.under(entry).next().is_some()
            || self
                .gitlinks
                .iter()
                .any(|g| g.starts_with(&format!("{entry}/")))
    }
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        let ft = e.file_type()?;
        if ft.is_dir() {
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            walk(root, &path, out)?;
        } else if ft.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

fn git_index_path(root: &Path) -> Option<PathBuf> {
    let dot = root.join(".git");
    if dot.is_dir() {
        return Some(dot.join("index"));
    }
    // A worktree or submodule: `.git` is a file naming the real Git directory.
    let text = std::fs::read_to_string(&dot).ok()?;
    let dir = text.trim().strip_prefix("gitdir:")?.trim();
    let dir = if Path::new(dir).is_absolute() {
        PathBuf::from(dir)
    } else {
        root.join(dir)
    };
    Some(dir.join("index"))
}

/// The commit `HEAD` points at, read natively from the Git directory.
pub fn head_commit(root: &Path) -> Option<String> {
    let index = git_index_path(root)?;
    let git = index.parent()?;
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let Some(r) = head.strip_prefix("ref: ") else {
        return Some(head.to_string());
    };
    // Worktrees keep refs in the common directory.
    let common = std::fs::read_to_string(git.join("commondir"))
        .ok()
        .map(|c| git.join(c.trim()))
        .unwrap_or_else(|| git.to_path_buf());
    for dir in [git, common.as_path()] {
        if let Ok(v) = std::fs::read_to_string(dir.join(r)) {
            return Some(v.trim().to_string());
        }
        if let Ok(packed) = std::fs::read_to_string(dir.join("packed-refs")) {
            if let Some(line) = packed.lines().find(|l| l.ends_with(&format!(" {r}"))) {
                return line.split(' ').next().map(str::to_string);
            }
        }
    }
    None
}

type IndexPaths = (BTreeSet<String>, BTreeSet<String>);

/// Reads a Git index (versions 2, 3 and 4). Returns tracked file paths and gitlinks.
pub fn parse_index(b: &[u8]) -> Result<IndexPaths, String> {
    if b.len() < 12 || &b[..4] != b"DIRC" {
        return Err("not a git index".into());
    }
    let version = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    if !(2..=4).contains(&version) {
        return Err(format!("unsupported index version {version}"));
    }
    let count = u32::from_be_bytes([b[8], b[9], b[10], b[11]]) as usize;
    let mut i = 12;
    let mut files = BTreeSet::new();
    let mut links = BTreeSet::new();
    let mut prev: Vec<u8> = Vec::new();
    for _ in 0..count {
        let start = i;
        if i + 62 > b.len() {
            return Err("truncated index entry".into());
        }
        let mode = u32::from_be_bytes([b[i + 24], b[i + 25], b[i + 26], b[i + 27]]);
        let flags = u16::from_be_bytes([b[i + 60], b[i + 61]]);
        i += 62;
        if version >= 3 && flags & 0x4000 != 0 {
            i += 2;
        }
        let path = if version == 4 {
            // Prefix compression: strip N bytes from the previous path, append the suffix.
            let mut n: usize = 0;
            let mut byte = b[i];
            i += 1;
            n = n.wrapping_add((byte & 0x7f) as usize);
            while byte & 0x80 != 0 {
                byte = b[i];
                i += 1;
                n = ((n + 1) << 7) | (byte & 0x7f) as usize;
            }
            let end = i + b[i..]
                .iter()
                .position(|c| *c == 0)
                .ok_or("unterminated path")?;
            let keep = prev.len().saturating_sub(n);
            let mut p = prev[..keep].to_vec();
            p.extend_from_slice(&b[i..end]);
            i = end + 1;
            p
        } else {
            let end = i + b[i..]
                .iter()
                .position(|c| *c == 0)
                .ok_or("unterminated path")?;
            let p = b[i..end].to_vec();
            // Entries are NUL-padded to a multiple of eight bytes.
            let len = end - start + 1;
            i = start + len.div_ceil(8) * 8;
            p
        };
        let stage = (flags >> 12) & 3;
        let s = String::from_utf8_lossy(&path).into_owned();
        if stage <= 1 {
            if mode & 0o170000 == 0o160000 {
                links.insert(s);
            } else {
                files.insert(s);
            }
        }
        prev = path;
    }
    Ok((files, links))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_v2(path: &str, mode: u32) -> Vec<u8> {
        let mut e = vec![0u8; 24];
        e.extend_from_slice(&mode.to_be_bytes());
        e.extend_from_slice(&[0u8; 12]); // uid, gid, size
        e.extend_from_slice(&[0u8; 20]); // sha
        e.extend_from_slice(&(path.len() as u16).to_be_bytes());
        e.extend_from_slice(path.as_bytes());
        let len = e.len() + 1;
        e.resize(len.div_ceil(8) * 8, 0);
        e
    }

    #[test]
    fn reads_v2_index_with_gitlinks() {
        let mut b = b"DIRC".to_vec();
        b.extend_from_slice(&2u32.to_be_bytes());
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend(entry_v2("Cargo.toml", 0o100644));
        b.extend(entry_v2("core/src/lib.rs", 0o100644));
        b.extend(entry_v2("vendor/xz", 0o160000));
        let (files, links) = parse_index(&b).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.contains("core/src/lib.rs"));
        assert!(links.contains("vendor/xz"));
    }

    #[test]
    fn reads_this_repository_index_when_present() {
        // The subsystem lives at `<repo>/.ecdev`; its repository is one level up.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let files = Files::scan(root).unwrap();
        if files.from_index {
            assert!(files.paths.iter().all(|p| !p.starts_with(".git/")));
        }
    }
}
