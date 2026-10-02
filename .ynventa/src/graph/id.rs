//! Stable node identity. PATH IS NOT IDENTITY: a node id is derived only from its namespace and
//! semantic key, so moving a node's files never changes its id, and ids from different
//! repositories never collide and never need rewriting when graphs merge.

use crate::digest::{hex, unhex, Sha256};

/// 128-bit node identity, printed `yn1:<32 hex>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub [u8; 16]);

/// Domain separator: the protocol generation of the identity function.
const DOMAIN: &[u8] = b"ynventa.node-id.v1";

/// Namespace shared by every repository for OSS donors identified by their upstream origin, so
/// that the same donor studied by two repositories is one node in a merged graph.
pub const OSS_NAMESPACE: &str = "oss";

impl NodeId {
    pub fn of(namespace: &str, key: &str) -> NodeId {
        let mut h = Sha256::new();
        h.field(DOMAIN);
        h.field(namespace.as_bytes());
        h.field(key.as_bytes());
        let d = h.finish();
        let mut id = [0u8; 16];
        id.copy_from_slice(&d[..16]);
        NodeId(id)
    }
    pub fn parse(s: &str) -> Option<NodeId> {
        let hex = s.strip_prefix("yn1:")?;
        let b = unhex(hex)?;
        (b.len() == 16).then(|| {
            let mut id = [0u8; 16];
            id.copy_from_slice(&b);
            NodeId(id)
        })
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "yn1:{}", hex(&self.0))
    }
}

/// Resolves an edge endpoint written in a declaration: `key` (this namespace) or
/// `namespace:key` (another repository's node).
pub fn resolve(reference: &str, namespace: &str) -> (String, String) {
    match reference.split_once(':') {
        Some((ns, key)) if !ns.is_empty() && !ns.contains('/') => (ns.to_string(), key.to_string()),
        _ => (namespace.to_string(), reference.to_string()),
    }
}

/// Normalises an upstream origin (URL or `owner/repo`) into the OSS identity key.
pub fn oss_key(origin: &str) -> Option<String> {
    let mut s = origin.trim().to_ascii_lowercase();
    for prefix in ["https://", "http://", "git+", "ssh://", "git://", "git@"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    s = s.replace("github.com:", "github.com/");
    let s = s.trim_end_matches('/').trim_end_matches(".git").to_string();
    if s.is_empty() || !s.contains('/') {
        return None;
    }
    if s.contains('.') && s.split('/').next().is_some_and(|h| h.contains('.')) {
        Some(s)
    } else {
        // `owner/repo` shorthand is GitHub by ecosystem convention.
        Some(format!("github.com/{s}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_namespace_and_key_only() {
        let a = NodeId::of("chronica", "substrate.storage");
        assert_eq!(a, NodeId::of("chronica", "substrate.storage"));
        assert_ne!(a, NodeId::of("mechatron", "substrate.storage"));
        assert_ne!(a, NodeId::of("chronica", "substrate.world"));
        // Field framing: no ambiguity between namespace and key boundaries.
        assert_ne!(NodeId::of("ab", "c"), NodeId::of("a", "bc"));
        assert_eq!(NodeId::parse(&a.to_string()), Some(a));
    }

    #[test]
    fn origins_normalise() {
        let k = Some("github.com/sqlite/sqlite".to_string());
        assert_eq!(oss_key("https://github.com/sqlite/sqlite.git"), k);
        assert_eq!(oss_key("sqlite/sqlite"), k);
        assert_eq!(oss_key("git@github.com:SQLite/sqlite"), k);
        assert_eq!(
            oss_key("https://gitlab.com/a/b/"),
            Some("gitlab.com/a/b".into())
        );
        assert_eq!(oss_key(""), None);
        assert_eq!(resolve("other:core", "me"), ("other".into(), "core".into()));
        assert_eq!(resolve("core", "me"), ("me".into(), "core".into()));
    }
}
