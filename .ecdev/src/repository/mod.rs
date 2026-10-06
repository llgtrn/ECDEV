//! The canonical physical repository shape: a plane architecture (kernel → substrate → domain →
//! adapter → application, with tools and tests beside them). Substrate owners are nodes inside
//! one `substrate/` role instead of one root each, and code is never bucketed by `crate/` or
//! `crates/`.

pub mod files;
pub mod shape;

use crate::schema::{NodeKind, Role};

pub struct RoleSpec {
    pub role: Role,
    /// The directory the role owns, relative to the repository root.
    pub dir: &'static str,
    /// Node kinds that may live in this role.
    pub kinds: &'static [NodeKind],
    /// Roles whose nodes this role's nodes may DEPEND_ON (itself included where listed).
    pub may_depend_on: &'static [Role],
    /// Whether the role may contain workspace build members.
    pub builds: bool,
}

use NodeKind as K;
use Role as R;

pub const ROLES: &[RoleSpec] = &[
    RoleSpec {
        role: R::Kernel,
        dir: "core",
        kinds: &[K::Kernel, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel],
        builds: true,
    },
    RoleSpec {
        role: R::Substrate,
        dir: "substrate",
        kinds: &[K::Substrate, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel, R::Substrate],
        builds: true,
    },
    RoleSpec {
        role: R::Domain,
        dir: "domain",
        kinds: &[K::Domain, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel, R::Substrate, R::Domain],
        builds: true,
    },
    RoleSpec {
        role: R::Adapter,
        dir: "adapter",
        kinds: &[K::Adapter, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel, R::Substrate, R::Domain, R::Adapter],
        builds: true,
    },
    RoleSpec {
        role: R::Application,
        dir: "apps",
        kinds: &[K::Application, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel, R::Substrate, R::Domain, R::Adapter],
        builds: true,
    },
    RoleSpec {
        role: R::Tool,
        dir: "tools",
        kinds: &[K::Tool, K::Test, K::Fixture],
        may_depend_on: &[R::Kernel, R::Substrate, R::Domain, R::Adapter, R::Tool],
        builds: true,
    },
    RoleSpec {
        role: R::Test,
        dir: "tests",
        kinds: &[K::Test, K::Fixture],
        may_depend_on: &[
            R::Kernel,
            R::Substrate,
            R::Domain,
            R::Adapter,
            R::Application,
            R::Tool,
            R::Test,
        ],
        builds: true,
    },
    RoleSpec {
        role: R::Research,
        dir: "research",
        kinds: &[K::Research],
        may_depend_on: &[],
        builds: false,
    },
    RoleSpec {
        role: R::Compat,
        dir: "compat",
        kinds: &[K::Compat],
        may_depend_on: &[
            R::Kernel,
            R::Substrate,
            R::Domain,
            R::Adapter,
            R::Application,
            R::Compat,
        ],
        builds: true,
    },
    RoleSpec {
        role: R::Governance,
        dir: ".ecdev",
        kinds: &[K::Governance, K::Test, K::Fixture],
        may_depend_on: &[],
        builds: false,
    },
    RoleSpec {
        role: R::Ci,
        dir: ".github",
        kinds: &[],
        may_depend_on: &[],
        builds: false,
    },
    RoleSpec {
        role: R::Toolchain,
        dir: ".cargo",
        kinds: &[],
        may_depend_on: &[],
        builds: false,
    },
    RoleSpec {
        role: R::Agent,
        dir: ".claude",
        kinds: &[],
        may_depend_on: &[],
        builds: false,
    },
];

/// Exact file names legal at the repository root.
pub const ROOT_FILES: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "CLAUDE.md",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "clippy.toml",
    "deny.toml",
    "package.json",
    "pnpm-workspace.yaml",
    "pnpm-lock.yaml",
    "tsconfig.json",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    // Secret scanning (ggshield) reads its configuration only from the repository root.
    ".gitguardian.yaml",
    "THIRD-PARTY-NOTICES.md",
    "NOTICE",
];

/// Root file-name prefixes that are legal (licences).
pub const ROOT_FILE_PREFIXES: &[&str] = &["LICENSE", "COPYING"];

/// Path segments that are never legal anywhere in a canonical path: generic containers that
/// carry no architectural meaning, and archive-by-directory. (Compatibility code lives only in
/// the `compat/` role, where every shim carries an expiry.)
pub const FORBIDDEN_SEGMENTS: &[&str] = &[
    "crates",
    "packages",
    "modules",
    "services",
    "components",
    "pkg",
    "vendor",
    "third_party",
    "third-party",
    "legacy",
    "deprecated",
    "archive",
    "old",
    "temporary",
    "tmp",
    "docs",
];

pub fn role_spec(role: Role) -> &'static RoleSpec {
    ROLES
        .iter()
        .find(|r| r.role == role)
        .expect("every role has a spec")
}

/// The role that owns a path, from its first segment.
pub fn role_of_path(path: &str) -> Option<Role> {
    let first = path.split('/').next()?;
    ROLES.iter().find(|r| r.dir == first).map(|r| r.role)
}

/// The home role of a node kind (tests and fixtures may also nest inside other roles).
pub fn home_role(kind: NodeKind) -> Option<Role> {
    Some(match kind {
        K::Kernel => R::Kernel,
        K::Substrate => R::Substrate,
        K::Domain => R::Domain,
        K::Adapter => R::Adapter,
        K::Application => R::Application,
        K::Tool => R::Tool,
        K::Test | K::Fixture => R::Test,
        K::Research => R::Research,
        K::Compat => R::Compat,
        K::Governance => R::Governance,
        _ => return None,
    })
}

/// The canonical path the shape assigns to a node of `kind` with semantic `key`: the role
/// directory, then the key's dotted segments. A key equal to the role directory is the role root.
pub fn derive_canonical_path(kind: NodeKind, key: &str) -> Option<String> {
    if kind == K::Repository {
        return Some(".".into());
    }
    let role = home_role(kind)?;
    let dir = role_spec(role).dir;
    if kind == K::Governance {
        return Some(dir.to_string());
    }
    let rest = key
        .strip_prefix(dir)
        .map(|r| r.trim_start_matches('.'))
        .unwrap_or(key);
    let rest = if kind == K::Fixture {
        format!("fixtures/{}", rest.trim_start_matches("fixtures."))
    } else {
        rest.to_string()
    };
    if rest.is_empty() {
        Some(dir.to_string())
    } else {
        Some(format!("{dir}/{}", rest.replace('.', "/")))
    }
}

fn legal_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Checks that `path` is a canonical location for a node of `kind`.
pub fn check_canonical_path(kind: NodeKind, path: &str) -> Result<(), String> {
    if kind == K::Repository {
        return if path == "." {
            Ok(())
        } else {
            Err("the repository node lives at `.`".into())
        };
    }
    if !crate::schema::is_physical(kind) {
        return if path.is_empty() {
            Ok(())
        } else {
            Err(format!("{kind} nodes are abstract and have no path"))
        };
    }
    let segs: Vec<&str> = path.split('/').collect();
    if kind == K::Governance {
        return if path == ".ecdev" {
            Ok(())
        } else {
            Err("the governance node lives at `.ecdev`".into())
        };
    }
    for s in &segs {
        if !legal_segment(s) {
            return Err(format!("segment `{s}` is not lowercase [a-z0-9_-]"));
        }
        if FORBIDDEN_SEGMENTS.contains(s) {
            return Err(format!("segment `{s}` is a forbidden generic container"));
        }
        if *s == "src" {
            return Err("`src` is a crate-internal directory, never a node path".into());
        }
    }
    let role =
        role_of_path(path).ok_or_else(|| format!("`{}` is not a role directory", segs[0]))?;
    let spec = role_spec(role);
    match kind {
        K::Test => {
            if segs.last() != Some(&"tests") && segs.first() != Some(&"tests") {
                return Err("a test suite lives at `tests/` or `<node>/tests`".into());
            }
        }
        K::Fixture => {
            let t = segs.iter().position(|s| *s == "tests");
            let f = segs.iter().position(|s| *s == "fixtures");
            if !matches!((t, f), (Some(t), Some(f)) if f == t + 1 && f + 1 < segs.len()) {
                return Err("fixtures live at `<..>/tests/fixtures/<name>`".into());
            }
        }
        _ => {
            if home_role(kind) != Some(role) {
                return Err(format!(
                    "{kind} nodes live under `{}/`, not `{}/`",
                    home_role(kind).map(|r| role_spec(r).dir).unwrap_or("?"),
                    spec.dir
                ));
            }
            let depth = segs.len();
            let max = match kind {
                K::Domain | K::Adapter => 4,
                _ => 2,
            };
            let root_ok = matches!(kind, K::Kernel | K::Adapter | K::Research);
            if depth > max || (depth == 1 && !root_ok) {
                return Err(format!(
                    "{kind} nodes live at `{}/<name>`{}",
                    spec.dir,
                    if max == 4 {
                        " or `<group>[/<subgroup>]/<name>`"
                    } else {
                        ""
                    }
                ));
            }
        }
    }
    if !spec.kinds.contains(&kind) {
        return Err(format!(
            "{kind} nodes may not live in the {} role",
            spec.role
        ));
    }
    Ok(())
}

pub fn root_file_allowed(name: &str) -> bool {
    ROOT_FILES.contains(&name) || ROOT_FILE_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Canonical rendering of the shape tables; part of the schema identity.
pub fn render_shape() -> String {
    let mut s = String::new();
    for r in ROLES {
        s.push_str(&format!(
            "role {} dir={} builds={} kinds={} depends={}\n",
            r.role,
            r.dir,
            r.builds,
            r.kinds
                .iter()
                .map(|k| k.wire())
                .collect::<Vec<_>>()
                .join(","),
            r.may_depend_on
                .iter()
                .map(|k| k.wire())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    s.push_str(&format!("root_files {}\n", ROOT_FILES.join(",")));
    s.push_str(&format!(
        "root_file_prefixes {}\n",
        ROOT_FILE_PREFIXES.join(",")
    ));
    s.push_str(&format!(
        "forbidden_segments {}\n",
        FORBIDDEN_SEGMENTS.join(",")
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths() {
        assert_eq!(
            derive_canonical_path(K::Kernel, "core").as_deref(),
            Some("core")
        );
        assert_eq!(
            derive_canonical_path(K::Kernel, "kernel").as_deref(),
            Some("core/kernel")
        );
        assert_eq!(
            derive_canonical_path(K::Substrate, "storage").as_deref(),
            Some("substrate/storage")
        );
        assert_eq!(
            derive_canonical_path(K::Adapter, "web.http").as_deref(),
            Some("adapter/web/http")
        );
        assert_eq!(
            derive_canonical_path(K::Test, "tests").as_deref(),
            Some("tests")
        );
        assert_eq!(
            derive_canonical_path(K::Fixture, "golden").as_deref(),
            Some("tests/fixtures/golden")
        );
        assert_eq!(
            derive_canonical_path(K::Governance, "governance").as_deref(),
            Some(".ecdev")
        );
        for (k, p) in [
            (K::Kernel, "core"),
            (K::Substrate, "substrate/storage"),
            (K::Domain, "domain/supply/procurement"),
            (K::Adapter, "adapter"),
            (K::Application, "apps/registry"),
            (K::Test, "tests"),
            (K::Test, "substrate/storage/tests"),
            (K::Fixture, "tests/fixtures/golden"),
            (K::Research, "research"),
            (K::Governance, ".ecdev"),
        ] {
            assert_eq!(check_canonical_path(k, p), Ok(()), "{k} {p}");
        }
        for (k, p) in [
            (K::Substrate, "storage"),
            (K::Substrate, "runtime/storage"),
            (K::Domain, "domains/ai"),
            (K::Kernel, "crates/core"),
            (K::Substrate, "substrate/Storage"),
            (K::Substrate, "substrate"),
            (K::Application, "apps/legacy"),
            (K::Fixture, "fixtures/x"),
            (K::Capability, "core"),
        ] {
            assert!(
                check_canonical_path(k, p).is_err(),
                "{k} {p} should be rejected"
            );
        }
    }
}
