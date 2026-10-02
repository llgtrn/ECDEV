//! Global donor identity: one upstream, one identity, however many repositories know it.
//!
//! A global donor key is derived only from a canonical upstream identity — a normalized
//! repository URL — and never from a name:
//!
//! * the scheme (`https://`, `ssh://`, `git+…`, the scp form `git@host:owner/repo`) and any
//!   user information are dropped, the host is lowercased and loses `www.` and a port;
//! * the fragment is dropped, `.git` and trailing `/` are stripped from the path;
//! * on a forge (GitHub, GitLab, Bitbucket, Codeberg) the key is `host/owner/repo` (GitLab:
//!   every group up to the repository), lowercased — forge owners and repositories are
//!   case-insensitive — and the rest of the URL (`/tree/main/…`, a query) is dropped;
//! * elsewhere the key is `host/path[?query]`, path and query case-sensitive;
//! * `owner/repo` with no host is GitHub shorthand; a bare word is a name, not an identity.
//!
//! Two keys are the same global donor only when they are equal or joined by a declared alias
//! with provenance. Records that merely look alike (same repository name, different host or
//! owner) stay distinct and are reported as AMBIGUOUS_DONOR_IDENTITY; nothing is ever merged by
//! name.
//!
//! Graph identities are not touched: a donor node's `NodeId` is still derived from the v1
//! origin key ([`legacy_key`]). Where the two keys differ, the legacy one is the global donor's
//! alias, with the declaration as its provenance.

use crate::graph::id::{oss_key, OSS_NAMESPACE};
use crate::graph::NodeId;
use std::collections::BTreeMap;

/// Forges whose identity is `host/owner/repo`.
pub const FORGES: &[&str] = &["github.com", "gitlab.com", "bitbucket.org", "codeberg.org"];

/// The global donor key of an upstream origin, or `None` when the origin names no upstream.
pub fn global_donor_key(origin: &str) -> Option<String> {
    let mut s = origin.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return None;
    }
    if s.len() >= 4 && s[..4].eq_ignore_ascii_case("git+") {
        s = &s[4..];
    }
    let lower = s.to_ascii_lowercase();
    let mut rest: Option<&str> = None;
    for scheme in ["https://", "http://", "ssh://", "git://", "ftp://"] {
        if lower.starts_with(scheme) {
            rest = Some(&s[scheme.len()..]);
            break;
        }
    }
    let had_scheme = rest.is_some();
    let rest = match rest {
        Some(r) => r.to_string(),
        None => match s.split_once(':') {
            // scp form: `git@host:owner/repo`.
            Some((user_host, path)) if user_host.contains('@') && !user_host.contains('/') => {
                format!("{}/{}", user_host, path.trim_start_matches('/'))
            }
            _ => s.to_string(),
        },
    };
    let rest = rest.split('#').next().unwrap_or("");
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, q),
        None => (rest, ""),
    };
    let (authority, path) = match rest.split_once('/') {
        Some((a, p)) => (a, p),
        None => (rest, ""),
    };
    let host_raw = authority.rsplit('@').next().unwrap_or(authority);
    let mut host = host_raw.to_ascii_lowercase();
    if let Some((h, port)) = host.split_once(':') {
        if port.chars().all(|c| c.is_ascii_digit()) {
            host = h.to_string();
        }
    }
    let mut path = path.trim_end_matches('/');
    path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    if !had_scheme && !host.contains('.') {
        // No host: `owner/repo` is GitHub shorthand; anything else is a name.
        let mut all = vec![host.as_str()];
        all.extend(&segments);
        return (all.len() == 2 && all.iter().all(|x| valid_segment(x)))
            .then(|| format!("github.com/{}", all.join("/").to_ascii_lowercase()));
    }
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    if FORGES.contains(&host.as_str()) && segments.len() >= 2 {
        let take = if host == "gitlab.com" {
            segments.iter().take_while(|x| **x != "-").count()
        } else {
            2
        };
        let repo: Vec<&str> = segments[..take.max(2)].to_vec();
        let mut key = format!("{host}/{}", repo.join("/")).to_ascii_lowercase();
        if let Some(k) = key.strip_suffix(".git") {
            key = k.to_string();
        }
        return Some(key);
    }
    let mut key = host;
    if !segments.is_empty() {
        key.push('/');
        key.push_str(&segments.join("/"));
    }
    if !query.is_empty() {
        key.push('?');
        key.push_str(query);
    }
    Some(key)
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The v1 key a donor node's `NodeId` is derived from (kept for identity stability).
pub fn legacy_key(origin: &str) -> Option<String> {
    oss_key(origin)
}

/// The node identity a global donor key has in the `oss` namespace.
pub fn node_id(key: &str) -> NodeId {
    NodeId::of(OSS_NAMESPACE, key)
}

/// The name a global donor looks like: its last path segment (the repository name on a forge),
/// lowercased, alphanumerics only. Two different keys with the same name look alike.
pub fn look_alike_name(key: &str) -> String {
    let path = key.split('?').next().unwrap_or(key);
    let last = path.rsplit('/').next().unwrap_or(path);
    last.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Union of keys joined by declared aliases. Every key not joined is its own class.
#[derive(Clone, Debug, Default)]
pub struct Aliases {
    parent: BTreeMap<String, String>,
}

impl Aliases {
    fn find(&self, k: &str) -> String {
        let mut cur = k.to_string();
        while let Some(p) = self.parent.get(&cur) {
            if *p == cur {
                break;
            }
            cur = p.clone();
        }
        cur
    }
    /// Joins two keys; the class is represented by its smallest key (deterministic).
    pub fn join(&mut self, a: &str, b: &str) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        self.parent.insert(hi, lo.clone());
        self.parent.entry(lo.clone()).or_insert(lo);
    }
    /// The canonical key of `k`'s class.
    pub fn canonical(&self, k: &str) -> String {
        self.find(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_normalise_to_one_identity() {
        let k = Some("github.com/serde-rs/serde".to_string());
        for o in [
            "https://github.com/serde-rs/serde",
            "https://github.com/serde-rs/serde.git",
            "https://github.com/serde-rs/serde/",
            "http://www.github.com/Serde-RS/Serde",
            "git@github.com:serde-rs/serde.git",
            "git+https://github.com/serde-rs/serde.git",
            "ssh://git@github.com:22/serde-rs/serde",
            "https://github.com/serde-rs/serde/tree/master/serde_derive",
            "https://github.com/serde-rs/serde#readme",
            "serde-rs/serde",
            "github.com/serde-rs/serde",
        ] {
            assert_eq!(global_donor_key(o), k, "{o}");
        }
        assert_eq!(
            global_donor_key("https://www.gnu.org/software/coreutils/"),
            Some("gnu.org/software/coreutils".into())
        );
        // Path case and the query matter off the forges.
        assert_eq!(
            global_donor_key("https://www.iso.org/search.html?q=10218-1"),
            Some("iso.org/search.html?q=10218-1".into())
        );
        assert_ne!(
            global_donor_key("https://www.iso.org/search.html?q=10218-1"),
            global_donor_key("https://www.iso.org/search.html?q=10218-2")
        );
        assert_eq!(
            global_donor_key("https://doi.org/10.1109/TSSC.1968.300136"),
            Some("doi.org/10.1109/TSSC.1968.300136".into())
        );
        // GitLab keeps its groups.
        assert_eq!(
            global_donor_key("https://gitlab.com/Group/Sub/Repo/-/tree/main"),
            Some("gitlab.com/group/sub/repo".into())
        );
        // A name is never an identity.
        assert_eq!(global_donor_key("serde"), None);
        assert_eq!(global_donor_key(""), None);
        assert_eq!(global_donor_key("not a url"), None);
        assert_eq!(
            look_alike_name("github.com/a/Nautilus_Trader"),
            "nautilustrader"
        );
    }

    #[test]
    fn graph_identities_keep_the_v1_key() {
        // The www. host is a different v1 key: the node id is unchanged and the global key is
        // the stronger one.
        let o = "https://www.gnu.org/software/coreutils";
        assert_eq!(legacy_key(o), Some("www.gnu.org/software/coreutils".into()));
        assert_eq!(
            global_donor_key(o),
            Some("gnu.org/software/coreutils".into())
        );
        let o = "https://github.com/serde-rs/serde";
        assert_eq!(legacy_key(o), global_donor_key(o));
    }

    #[test]
    fn aliases_join_classes_deterministically() {
        let mut a = Aliases::default();
        a.join("github.com/b/x", "github.com/a/x");
        a.join("github.com/c/x", "github.com/b/x");
        assert_eq!(a.canonical("github.com/c/x"), "github.com/a/x");
        assert_eq!(a.canonical("github.com/z/x"), "github.com/z/x");
    }
}
