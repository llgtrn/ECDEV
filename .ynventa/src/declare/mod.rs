//! Authoritative repository declarations:
//! `.ynventa/declared/{repository,donors,migration,technologies,organism}.rs`.
//!
//! Each file is one Rust constant expression written against `declare::decl`. rustc checks the
//! types when the subsystem builds; this module reads the same bytes at run time, so that one
//! canonical verifier reads every repository identically.
//!
//! The grammar is versioned by the subsystem: [`load`] accepts only the current grammar. Earlier
//! grammars (donor capabilities and technologies without `maps_to`/`norl`, no `organism.rs`, an
//! organism without `promotions`) are read only by [`load_previous`], which `ynventa migrate
//! schema` uses to rewrite a repository into the current grammar; they are never accepted
//! silently.

pub mod decl;
pub mod model;

pub use model::*;

use crate::formats::rust::{self as r, Expr, Kind};
use crate::schema::{
    BackendKind, Concept, Dimension, DonorState, Ecosystem, EdgeKind, ExceptionKind, ExpiryKind,
    GrowthState, NodeKind, NodeLifecycle, PromotionImplementation, PromotionState, ProofKind,
    Scope, ShimKind, TechnologyKind, TechnologyLifecycle, WaveStatus,
};
use std::path::{Path, PathBuf};

pub const DECLARED_DIR: &str = ".ynventa/declared";
pub const REPOSITORY_FILE: &str = "repository.rs";
pub const DONORS_FILE: &str = "donors.rs";
pub const MIGRATION_FILE: &str = "migration.rs";
pub const TECHNOLOGIES_FILE: &str = "technologies.rs";
pub const ORGANISM_FILE: &str = "organism.rs";

/// Every declaration file, in canonical order.
pub const DECLARATION_FILES: &[&str] = &[
    REPOSITORY_FILE,
    DONORS_FILE,
    MIGRATION_FILE,
    TECHNOLOGIES_FILE,
    ORGANISM_FILE,
];

#[derive(Debug, Clone, PartialEq)]
pub struct DeclError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for DeclError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

pub fn declared_dir(root: &Path) -> PathBuf {
    root.join(DECLARED_DIR)
}

/// Which grammar a reading accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grammar {
    Current,
    /// The earlier grammars: no `maps_to`/`norl` fields, no `organism.rs`, or an organism
    /// without `promotions`.
    Previous,
}

/// Loads the declaration files of the repository at `root` in the current grammar.
pub fn load(root: &Path) -> Result<Declaration, DeclError> {
    load_in(root, Grammar::Current).map_err(|mut e| {
        if load_in(root, Grammar::Previous).is_ok() {
            e.message.push_str(&format!(
                " (the declarations use the previous grammar: run `ynventa migrate schema --root {}`)",
                root.display()
            ));
        }
        e
    })
}

/// Loads declarations written in the previous grammar (or the current one): the reading
/// `migrate schema` rewrites from. Missing `maps_to` is `None`, missing `norl` is
/// `NorlRelevance::Unresolved`, a missing `organism.rs` is the empty organism, missing
/// `promotions` are none.
pub fn load_previous(root: &Path) -> Result<Declaration, DeclError> {
    load_in(root, Grammar::Previous)
}

fn load_in(root: &Path, g: Grammar) -> Result<Declaration, DeclError> {
    let dir = declared_dir(root);
    let read = |name: &str| -> Result<(String, String), DeclError> {
        let rel = format!("{DECLARED_DIR}/{name}");
        std::fs::read_to_string(dir.join(name))
            .map(|t| (rel.clone(), t))
            .map_err(|e| DeclError {
                file: rel,
                line: 0,
                message: format!("cannot read: {e}"),
            })
    };
    let (f1, t1) = read(REPOSITORY_FILE)?;
    let (f2, t2) = read(DONORS_FILE)?;
    let (f3, t3) = read(MIGRATION_FILE)?;
    let (f4, t4) = read(TECHNOLOGIES_FILE)?;
    let organism = match (read(ORGANISM_FILE), g) {
        (Ok((f5, t5)), _) => parse_organism_in(&t5, g).map_err(|e| e.file(&f5))?,
        (Err(_), Grammar::Previous) => Organism::default(),
        (Err(e), Grammar::Current) => return Err(e),
    };
    Ok(Declaration {
        repository: parse_repository(&t1).map_err(|e| e.file(&f1))?,
        donors: parse_donors_in(&t2, g).map_err(|e| e.file(&f2))?,
        migration: parse_migration(&t3).map_err(|e| e.file(&f3))?,
        technologies: parse_technologies_in(&t4, g).map_err(|e| e.file(&f4))?,
        organism,
    })
}

/// Rewrites the declarations of `root` into the current grammar: every file is re-rendered
/// canonically when its reading changes, `organism.rs` is created empty when missing. Nothing
/// is lowered or raised: a donor capability without a mapping gets `maps_to: None` and
/// `norl: NorlRelevance::Unresolved`, so a claim the new ladder no longer supports is reported
/// by `verify`. Returns the files written.
pub fn migrate_schema(root: &Path) -> Result<Vec<String>, String> {
    let d = load_previous(root).map_err(|e| e.to_string())?;
    store_report(root, &d).map_err(|e| e.to_string())
}

/// Writes the declaration files canonically, each only when what it declares changed.
///
/// A file whose current text already parses to the value being stored is left byte-identical,
/// whatever its layout: storing an unchanged declaration (as every `migrate apply` does for the
/// files a wave does not touch) never creates diff noise. A changed, missing or unreadable file
/// (including one in the previous grammar) is rewritten in the canonical rendering.
pub fn store(root: &Path, d: &Declaration) -> std::io::Result<()> {
    store_report(root, d).map(|_| ())
}

/// [`store`], returning the declaration files it wrote.
pub fn store_report(root: &Path, d: &Declaration) -> std::io::Result<Vec<String>> {
    let dir = declared_dir(root);
    std::fs::create_dir_all(&dir)?;
    let wrote = [
        store_file(
            &dir.join(REPOSITORY_FILE),
            &d.repository,
            parse_repository,
            render_repository,
        )?,
        store_file(
            &dir.join(DONORS_FILE),
            &d.donors[..],
            parse_donors,
            render_donors,
        )?,
        store_file(
            &dir.join(MIGRATION_FILE),
            &d.migration,
            parse_migration,
            render_migration,
        )?,
        store_file(
            &dir.join(TECHNOLOGIES_FILE),
            &d.technologies[..],
            parse_technologies,
            render_technologies,
        )?,
        store_file(
            &dir.join(ORGANISM_FILE),
            &d.organism,
            parse_organism,
            render_organism,
        )?,
    ];
    Ok(DECLARATION_FILES
        .iter()
        .zip(wrote)
        .filter(|(_, w)| *w)
        .map(|(f, _)| format!("{DECLARED_DIR}/{f}"))
        .collect())
}

/// Writes `value` to `path` unless the file there already declares exactly `value`; returns
/// whether it wrote.
fn store_file<T: PartialEq + ?Sized, O: std::borrow::Borrow<T>>(
    path: &Path,
    value: &T,
    parse: fn(&str) -> Res<O>,
    render: fn(&T) -> String,
) -> std::io::Result<bool> {
    if let Ok(text) = std::fs::read_to_string(path) {
        if parse(&text).is_ok_and(|v| v.borrow() == value) {
            return Ok(false);
        }
    }
    std::fs::write(path, render(value)).map(|_| true)
}

// ---------------------------------------------------------------------------------------------
// Reading.

pub struct Err0 {
    line: usize,
    message: String,
}

impl Err0 {
    fn file(self, f: &str) -> DeclError {
        DeclError {
            file: f.to_string(),
            line: self.line,
            message: self.message,
        }
    }
}

type Res<T> = Result<T, Err0>;

fn fail<T>(e: &Expr, m: impl Into<String>) -> Res<T> {
    Err(Err0 {
        line: e.line,
        message: m.into(),
    })
}

struct Fields<'a> {
    at: &'a Expr,
    items: &'a [(String, Expr)],
    used: Vec<&'a str>,
}

impl<'a> Fields<'a> {
    fn of(e: &'a Expr, name: &str) -> Res<Self> {
        match &e.kind {
            Kind::Struct(n, items) if n == name => Ok(Fields {
                at: e,
                items,
                used: Vec::new(),
            }),
            Kind::Struct(n, _) => fail(e, format!("expected `{name} {{ .. }}`, found `{n}`")),
            _ => fail(e, format!("expected `{name} {{ .. }}`")),
        }
    }
    fn get(&mut self, f: &'a str) -> Res<&'a Expr> {
        self.used.push(f);
        match self.items.iter().find(|(n, _)| n == f) {
            Some((_, v)) => Ok(v),
            None => fail(self.at, format!("missing field `{f}`")),
        }
    }
    /// A field the previous grammar did not have: optional there, required in the current one.
    fn added(&mut self, f: &'a str, g: Grammar) -> Res<Option<&'a Expr>> {
        if g == Grammar::Previous && !self.items.iter().any(|(n, _)| n == f) {
            return Ok(None);
        }
        self.get(f).map(Some)
    }
    fn string(&mut self, f: &'a str) -> Res<String> {
        let v = self.get(f)?;
        string(v)
    }
    fn strings(&mut self, f: &'a str) -> Res<Vec<String>> {
        let v = self.get(f)?;
        list(v)?.iter().map(string).collect()
    }
    fn done(self) -> Res<()> {
        for (n, v) in self.items {
            if !self.used.contains(&n.as_str()) {
                return fail(v, format!("unknown field `{n}`"));
            }
        }
        Ok(())
    }
}

fn string(e: &Expr) -> Res<String> {
    match &e.kind {
        Kind::Str(s) => Ok(s.clone()),
        _ => fail(e, "expected a string literal"),
    }
}

fn list(e: &Expr) -> Res<&[Expr]> {
    match &e.kind {
        Kind::List(items) => Ok(items),
        _ => fail(e, "expected a slice `&[..]`"),
    }
}

fn boolean(e: &Expr) -> Res<bool> {
    match &e.kind {
        Kind::Bool(b) => Ok(*b),
        _ => fail(e, "expected `true` or `false`"),
    }
}

fn opt_string(e: &Expr) -> Res<Option<String>> {
    match &e.kind {
        Kind::Path(p) if p.len() == 1 && p[0] == "None" => Ok(None),
        Kind::Call(p, args) if p.len() == 1 && p[0] == "Some" && args.len() == 1 => {
            Ok(Some(string(&args[0])?))
        }
        _ => fail(e, "expected `None` or `Some(\"..\")`"),
    }
}

/// Reads `Enum::Variant` into a vocabulary word.
fn variant<T>(e: &Expr, enum_name: &str, from: fn(&str) -> Option<T>) -> Res<T> {
    match &e.kind {
        Kind::Path(p) if p.len() == 2 && p[0] == enum_name => from(&p[1]).map_or_else(
            || fail(e, format!("`{}` is not a {enum_name}", p.join("::"))),
            Ok,
        ),
        _ => fail(e, format!("expected `{enum_name}::<Variant>`")),
    }
}

/// Reads `Enum::Variant("arg")`.
fn tagged<'a>(e: &'a Expr, enum_name: &str) -> Res<(&'a str, String)> {
    match &e.kind {
        Kind::Call(p, args) if p.len() == 2 && p[0] == enum_name && args.len() == 1 => {
            Ok((&p[1], string(&args[0])?))
        }
        _ => fail(e, format!("expected `{enum_name}::<Variant>(\"..\")`")),
    }
}

fn parse_file(text: &str) -> Res<Expr> {
    r::parse(text).map_err(|e| Err0 {
        line: e.line,
        message: e.message,
    })
}

pub fn parse_repository(text: &str) -> Res<Repository> {
    let e = parse_file(text)?;
    let mut f = Fields::of(&e, "Repository")?;
    let system = f.string("system")?;
    let shard = f.string("shard")?;
    let name = f.string("name")?;
    let origin = f.string("origin")?;
    let nodes = list(f.get("nodes")?)?
        .iter()
        .map(|n| {
            let mut f = Fields::of(n, "Node")?;
            let node = Node {
                key: f.string("key")?,
                kind: variant(f.get("kind")?, "NodeKind", NodeKind::from_variant)?,
                concept: variant(f.get("concept")?, "Concept", Concept::from_variant)?,
                name: f.string("name")?,
                path: f.string("path")?,
                canonical_path: f.string("canonical_path")?,
                lifecycle: variant(
                    f.get("lifecycle")?,
                    "NodeLifecycle",
                    NodeLifecycle::from_variant,
                )?,
                provides: f.strings("provides")?,
                requires: f.strings("requires")?,
                reuses: f.strings("reuses")?,
                inputs: f.strings("inputs")?,
                outputs: f.strings("outputs")?,
                lineage: f.strings("lineage")?,
            };
            f.done()?;
            Ok(node)
        })
        .collect::<Res<Vec<_>>>()?;
    let edges = list(f.get("edges")?)?
        .iter()
        .map(|x| {
            let mut f = Fields::of(x, "Edge")?;
            let edge = Edge {
                from: f.string("from")?,
                to: f.string("to")?,
                kind: variant(f.get("kind")?, "EdgeKind", EdgeKind::from_variant)?,
                scope: variant(f.get("scope")?, "Scope", Scope::from_variant)?,
            };
            f.done()?;
            Ok(edge)
        })
        .collect::<Res<Vec<_>>>()?;
    f.done()?;
    Ok(Repository {
        system,
        shard,
        name,
        origin,
        nodes,
        edges,
    })
}

fn norl_relevance(e: &Expr) -> Res<NorlRelevance> {
    match &e.kind {
        Kind::Path(p) if p.len() == 2 && p[0] == "NorlRelevance" && p[1] == "Unresolved" => {
            Ok(NorlRelevance::Unresolved)
        }
        _ => match tagged(e, "NorlRelevance")? {
            ("Feeds", k) => Ok(NorlRelevance::Feeds(k)),
            ("NotRelevant", r) => Ok(NorlRelevance::NotRelevant(r)),
            (v, _) => fail(e, format!("`NorlRelevance::{v}` is not a Norl relevance")),
        },
    }
}

fn render_norl(n: &NorlRelevance) -> Expr {
    match n {
        NorlRelevance::Unresolved => r::path("NorlRelevance::Unresolved"),
        NorlRelevance::Feeds(k) => r::call("NorlRelevance::Feeds", vec![r::s(k)]),
        NorlRelevance::NotRelevant(x) => r::call("NorlRelevance::NotRelevant", vec![r::s(x)]),
    }
}

pub fn parse_donors(text: &str) -> Res<Vec<Donor>> {
    parse_donors_in(text, Grammar::Current)
}

fn parse_donors_in(text: &str, g: Grammar) -> Res<Vec<Donor>> {
    let e = parse_file(text)?;
    list(&e)?
        .iter()
        .map(|d| {
            let mut f = Fields::of(d, "Donor")?;
            let exception = {
                let x = f.get("exception")?;
                match &x.kind {
                    Kind::Path(p) if p.len() == 2 && p[0] == "Exception" && p[1] == "None" => None,
                    _ => {
                        let (v, arg) = tagged(x, "Exception")?;
                        let kind = ExceptionKind::from_variant(v).map_or_else(
                            || fail(x, format!("`Exception::{v}` is not an exception")),
                            Ok,
                        )?;
                        Some((kind, arg))
                    }
                }
            };
            let donor = Donor {
                key: f.string("key")?,
                name: f.string("name")?,
                origin: f.string("origin")?,
                license: f.string("license")?,
                claimed: variant(f.get("claimed")?, "DonorState", DonorState::from_variant)?,
                exception,
                packages: list(f.get("packages")?)?
                    .iter()
                    .map(|p| {
                        let mut f = Fields::of(p, "Package")?;
                        let pkg = Package {
                            ecosystem: variant(
                                f.get("ecosystem")?,
                                "Ecosystem",
                                Ecosystem::from_variant,
                            )?,
                            name: f.string("name")?,
                        };
                        f.done()?;
                        Ok(pkg)
                    })
                    .collect::<Res<Vec<_>>>()?,
                source_paths: f.strings("source_paths")?,
                capabilities: list(f.get("capabilities")?)?
                    .iter()
                    .map(|c| {
                        let mut f = Fields::of(c, "Capability")?;
                        let cap = Capability {
                            key: f.string("key")?,
                            required: boolean(f.get("required")?)?,
                            spec: f.string("spec")?,
                            replacement: opt_string(f.get("replacement")?)?,
                            maps_to: match f.added("maps_to", g)? {
                                Some(x) => opt_string(x)?,
                                None => None,
                            },
                            norl: match f.added("norl", g)? {
                                Some(x) => norl_relevance(x)?,
                                None => NorlRelevance::Unresolved,
                            },
                            proofs: list(f.get("proofs")?)?
                                .iter()
                                .map(|p| {
                                    let mut f = Fields::of(p, "Proof")?;
                                    let proof = Proof {
                                        kind: variant(
                                            f.get("kind")?,
                                            "ProofKind",
                                            ProofKind::from_variant,
                                        )?,
                                        locator: f.string("locator")?,
                                    };
                                    f.done()?;
                                    Ok(proof)
                                })
                                .collect::<Res<Vec<_>>>()?,
                        };
                        f.done()?;
                        Ok(cap)
                    })
                    .collect::<Res<Vec<_>>>()?,
                cutover: opt_string(f.get("cutover")?)?,
                provenance: f.strings("provenance")?,
            };
            f.done()?;
            Ok(donor)
        })
        .collect()
}

pub fn parse_migration(text: &str) -> Res<Migration> {
    let e = parse_file(text)?;
    let mut f = Fields::of(&e, "Migration")?;
    let waves = list(f.get("waves")?)?
        .iter()
        .map(|w| {
            let mut f = Fields::of(w, "Wave")?;
            let wave = Wave {
                key: f.string("key")?,
                status: variant(f.get("status")?, "WaveStatus", WaveStatus::from_variant)?,
                nodes: f.strings("nodes")?,
            };
            f.done()?;
            Ok(wave)
        })
        .collect::<Res<Vec<_>>>()?;
    let shims = list(f.get("shims")?)?
        .iter()
        .map(|s| {
            let mut f = Fields::of(s, "Shim")?;
            let key = f.string("key")?;
            let kind = variant(f.get("kind")?, "ShimKind", ShimKind::from_variant)?;
            let path = f.string("path")?;
            let serves = f.string("serves")?;
            let x = f.get("expires")?;
            let (v, arg) = tagged(x, "Expiry")?;
            let expiry = ExpiryKind::from_variant(v)
                .map_or_else(|| fail(x, format!("`Expiry::{v}` is not an expiry")), Ok)?;
            f.done()?;
            Ok(Shim {
                key,
                kind,
                path,
                serves,
                expires: (expiry, arg),
            })
        })
        .collect::<Res<Vec<_>>>()?;
    f.done()?;
    Ok(Migration { waves, shims })
}

// ---------------------------------------------------------------------------------------------
// Rendering.

const HEADER: &str =
    "// Ynventa declaration (protocol v1). Authoritative and typed: rustc checks it against
// `ynventa::declare::decl`, and every `ynventa` verifier reads it at run time.
// `ynventa` rewrites this file canonically; comments inside the expression are not kept.
";

fn en<T: Copy>(name: &str, v: T, variant: fn(T) -> &'static str) -> Expr {
    r::path(&format!("{name}::{}", variant(v)))
}

pub fn render_repository(repo: &Repository) -> String {
    let nodes = repo
        .nodes
        .iter()
        .map(|n| {
            r::st(
                "Node",
                vec![
                    ("key", r::s(&n.key)),
                    ("kind", en("NodeKind", n.kind, NodeKind::variant)),
                    ("concept", en("Concept", n.concept, Concept::variant)),
                    ("name", r::s(&n.name)),
                    ("path", r::s(&n.path)),
                    ("canonical_path", r::s(&n.canonical_path)),
                    (
                        "lifecycle",
                        en("NodeLifecycle", n.lifecycle, NodeLifecycle::variant),
                    ),
                    ("provides", r::strs(&n.provides)),
                    ("requires", r::strs(&n.requires)),
                    ("reuses", r::strs(&n.reuses)),
                    ("inputs", r::strs(&n.inputs)),
                    ("outputs", r::strs(&n.outputs)),
                    ("lineage", r::strs(&n.lineage)),
                ],
            )
        })
        .collect();
    let edges = repo
        .edges
        .iter()
        .map(|e| {
            r::st(
                "Edge",
                vec![
                    ("from", r::s(&e.from)),
                    ("to", r::s(&e.to)),
                    ("kind", en("EdgeKind", e.kind, EdgeKind::variant)),
                    ("scope", en("Scope", e.scope, Scope::variant)),
                ],
            )
        })
        .collect();
    let e = r::st(
        "Repository",
        vec![
            ("system", r::s(&repo.system)),
            ("shard", r::s(&repo.shard)),
            ("name", r::s(&repo.name)),
            ("origin", r::s(&repo.origin)),
            ("nodes", r::list(nodes)),
            ("edges", r::list(edges)),
        ],
    );
    format!("{HEADER}{}", r::render(&e))
}

pub fn render_donors(donors: &[Donor]) -> String {
    let items = donors
        .iter()
        .map(|d| {
            r::st(
                "Donor",
                vec![
                    ("key", r::s(&d.key)),
                    ("name", r::s(&d.name)),
                    ("origin", r::s(&d.origin)),
                    ("license", r::s(&d.license)),
                    ("claimed", en("DonorState", d.claimed, DonorState::variant)),
                    (
                        "exception",
                        match &d.exception {
                            None => r::path("Exception::None"),
                            Some((k, a)) => {
                                r::call(&format!("Exception::{}", k.variant()), vec![r::s(a)])
                            }
                        },
                    ),
                    (
                        "packages",
                        r::list(
                            d.packages
                                .iter()
                                .map(|p| {
                                    r::st(
                                        "Package",
                                        vec![
                                            (
                                                "ecosystem",
                                                en("Ecosystem", p.ecosystem, Ecosystem::variant),
                                            ),
                                            ("name", r::s(&p.name)),
                                        ],
                                    )
                                })
                                .collect(),
                        ),
                    ),
                    ("source_paths", r::strs(&d.source_paths)),
                    (
                        "capabilities",
                        r::list(
                            d.capabilities
                                .iter()
                                .map(|c| {
                                    r::st(
                                        "Capability",
                                        vec![
                                            ("key", r::s(&c.key)),
                                            ("required", r::boolean(c.required)),
                                            ("spec", r::s(&c.spec)),
                                            ("replacement", r::opt(&c.replacement)),
                                            ("maps_to", r::opt(&c.maps_to)),
                                            ("norl", render_norl(&c.norl)),
                                            (
                                                "proofs",
                                                r::list(
                                                    c.proofs
                                                        .iter()
                                                        .map(|p| {
                                                            r::st(
                                                                "Proof",
                                                                vec![
                                                                    (
                                                                        "kind",
                                                                        en(
                                                                            "ProofKind",
                                                                            p.kind,
                                                                            ProofKind::variant,
                                                                        ),
                                                                    ),
                                                                    ("locator", r::s(&p.locator)),
                                                                ],
                                                            )
                                                        })
                                                        .collect(),
                                                ),
                                            ),
                                        ],
                                    )
                                })
                                .collect(),
                        ),
                    ),
                    ("cutover", r::opt(&d.cutover)),
                    ("provenance", r::strs(&d.provenance)),
                ],
            )
        })
        .collect();
    format!("{HEADER}{}", r::render(&r::list(items)))
}

pub fn render_migration(m: &Migration) -> String {
    let waves = m
        .waves
        .iter()
        .map(|w| {
            r::st(
                "Wave",
                vec![
                    ("key", r::s(&w.key)),
                    ("status", en("WaveStatus", w.status, WaveStatus::variant)),
                    ("nodes", r::strs(&w.nodes)),
                ],
            )
        })
        .collect();
    let shims = m
        .shims
        .iter()
        .map(|s| {
            r::st(
                "Shim",
                vec![
                    ("key", r::s(&s.key)),
                    ("kind", en("ShimKind", s.kind, ShimKind::variant)),
                    ("path", r::s(&s.path)),
                    ("serves", r::s(&s.serves)),
                    (
                        "expires",
                        r::call(
                            &format!("Expiry::{}", s.expires.0.variant()),
                            vec![r::s(&s.expires.1)],
                        ),
                    ),
                ],
            )
        })
        .collect();
    let e = r::st(
        "Migration",
        vec![("waves", r::list(waves)), ("shims", r::list(shims))],
    );
    format!("{HEADER}{}", r::render(&e))
}

fn parse_proofs(e: &Expr) -> Res<Vec<Proof>> {
    list(e)?
        .iter()
        .map(|p| {
            let mut f = Fields::of(p, "Proof")?;
            let proof = Proof {
                kind: variant(f.get("kind")?, "ProofKind", ProofKind::from_variant)?,
                locator: f.string("locator")?,
            };
            f.done()?;
            Ok(proof)
        })
        .collect()
}

pub fn parse_technologies(text: &str) -> Res<Vec<Technology>> {
    parse_technologies_in(text, Grammar::Current)
}

fn parse_technologies_in(text: &str, g: Grammar) -> Res<Vec<Technology>> {
    let e = parse_file(text)?;
    list(&e)?
        .iter()
        .map(|t| {
            let mut f = Fields::of(t, "Technology")?;
            let tech = Technology {
                key: f.string("key")?,
                name: f.string("name")?,
                kind: variant(
                    f.get("kind")?,
                    "TechnologyKind",
                    TechnologyKind::from_variant,
                )?,
                claimed: variant(
                    f.get("claimed")?,
                    "TechnologyLifecycle",
                    TechnologyLifecycle::from_variant,
                )?,
                purpose: f.string("purpose")?,
                implements: f.strings("implements")?,
                node: f.string("node")?,
                sources: f.strings("sources")?,
                invariants: f.strings("invariants")?,
                proofs: parse_proofs(f.get("proofs")?)?,
                lineage: f.strings("lineage")?,
                relations: list(f.get("relations")?)?
                    .iter()
                    .map(|x| {
                        let mut f = Fields::of(x, "Relation")?;
                        let r = Relation {
                            kind: variant(f.get("kind")?, "EdgeKind", EdgeKind::from_variant)?,
                            target: f.string("target")?,
                        };
                        f.done()?;
                        Ok(r)
                    })
                    .collect::<Res<Vec<_>>>()?,
                norl: match f.added("norl", g)? {
                    Some(x) => norl_relevance(x)?,
                    None => NorlRelevance::Unresolved,
                },
                claims: list(f.get("claims")?)?
                    .iter()
                    .map(|x| {
                        let mut f = Fields::of(x, "Improvement")?;
                        let c = Improvement {
                            dimension: variant(
                                f.get("dimension")?,
                                "Dimension",
                                Dimension::from_variant,
                            )?,
                            baseline: f.string("baseline")?,
                            workload: f.string("workload")?,
                            statement: f.string("statement")?,
                            evidence: parse_proofs(f.get("evidence")?)?,
                        };
                        f.done()?;
                        Ok(c)
                    })
                    .collect::<Res<Vec<_>>>()?,
            };
            f.done()?;
            Ok(tech)
        })
        .collect()
}

fn render_proofs(p: &[Proof]) -> Expr {
    r::list(
        p.iter()
            .map(|p| {
                r::st(
                    "Proof",
                    vec![
                        ("kind", en("ProofKind", p.kind, ProofKind::variant)),
                        ("locator", r::s(&p.locator)),
                    ],
                )
            })
            .collect(),
    )
}

pub fn render_technologies(ts: &[Technology]) -> String {
    let items = ts
        .iter()
        .map(|t| {
            r::st(
                "Technology",
                vec![
                    ("key", r::s(&t.key)),
                    ("name", r::s(&t.name)),
                    (
                        "kind",
                        en("TechnologyKind", t.kind, TechnologyKind::variant),
                    ),
                    (
                        "claimed",
                        en(
                            "TechnologyLifecycle",
                            t.claimed,
                            TechnologyLifecycle::variant,
                        ),
                    ),
                    ("purpose", r::s(&t.purpose)),
                    ("implements", r::strs(&t.implements)),
                    ("node", r::s(&t.node)),
                    ("sources", r::strs(&t.sources)),
                    ("invariants", r::strs(&t.invariants)),
                    ("proofs", render_proofs(&t.proofs)),
                    ("lineage", r::strs(&t.lineage)),
                    (
                        "relations",
                        r::list(
                            t.relations
                                .iter()
                                .map(|x| {
                                    r::st(
                                        "Relation",
                                        vec![
                                            ("kind", en("EdgeKind", x.kind, EdgeKind::variant)),
                                            ("target", r::s(&x.target)),
                                        ],
                                    )
                                })
                                .collect(),
                        ),
                    ),
                    ("norl", render_norl(&t.norl)),
                    (
                        "claims",
                        r::list(
                            t.claims
                                .iter()
                                .map(|c| {
                                    r::st(
                                        "Improvement",
                                        vec![
                                            (
                                                "dimension",
                                                en("Dimension", c.dimension, Dimension::variant),
                                            ),
                                            ("baseline", r::s(&c.baseline)),
                                            ("workload", r::s(&c.workload)),
                                            ("statement", r::s(&c.statement)),
                                            ("evidence", render_proofs(&c.evidence)),
                                        ],
                                    )
                                })
                                .collect(),
                        ),
                    ),
                ],
            )
        })
        .collect();
    format!("{HEADER}{}", r::render(&r::list(items)))
}

pub fn parse_organism(text: &str) -> Res<Organism> {
    parse_organism_in(text, Grammar::Current)
}

fn parse_organism_in(text: &str, g: Grammar) -> Res<Organism> {
    let e = parse_file(text)?;
    let mut f = Fields::of(&e, "Organism")?;
    let capabilities = list(f.get("capabilities")?)?
        .iter()
        .map(|c| {
            let mut f = Fields::of(c, "OrganismCapability")?;
            let cap = OrganismCapability {
                key: f.string("key")?,
                organ: f.string("organ")?,
                claimed: variant(f.get("claimed")?, "GrowthState", GrowthState::from_variant)?,
                backend: opt_string(f.get("backend")?)?,
                evaluations: f.strings("evaluations")?,
            };
            f.done()?;
            Ok(cap)
        })
        .collect::<Res<Vec<_>>>()?;
    let backends = list(f.get("backends")?)?
        .iter()
        .map(|b| {
            let mut f = Fields::of(b, "Backend")?;
            let backend = Backend {
                key: f.string("key")?,
                kind: variant(f.get("kind")?, "BackendKind", BackendKind::from_variant)?,
                node: f.string("node")?,
                donor: opt_string(f.get("donor")?)?,
                weight: f.string("weight")?,
            };
            f.done()?;
            Ok(backend)
        })
        .collect::<Res<Vec<_>>>()?;
    let promotions = match f.added("promotions", g)? {
        None => Vec::new(),
        Some(x) => list(x)?
            .iter()
            .map(|p| {
                let mut f = Fields::of(p, "Promotion")?;
                let promotion = Promotion {
                    key: f.string("key")?,
                    source_repository: f.string("source_repository")?,
                    source_node: f.string("source_node")?,
                    source_capability: opt_string(f.get("source_capability")?)?,
                    source_technology: opt_string(f.get("source_technology")?)?,
                    source_commit: f.string("source_commit")?,
                    source_evidence: f.strings("source_evidence")?,
                    destination_capability: f.string("destination_capability")?,
                    destination_node: opt_string(f.get("destination_node")?)?,
                    relation: variant(f.get("relation")?, "EdgeKind", EdgeKind::from_variant)?,
                    implementation: variant(
                        f.get("implementation")?,
                        "PromotionImplementation",
                        PromotionImplementation::from_variant,
                    )?,
                    claimed: variant(
                        f.get("claimed")?,
                        "PromotionState",
                        PromotionState::from_variant,
                    )?,
                    reason: f.string("reason")?,
                    transformation: f.string("transformation")?,
                };
                f.done()?;
                Ok(promotion)
            })
            .collect::<Res<Vec<_>>>()?,
    };
    f.done()?;
    Ok(Organism {
        capabilities,
        backends,
        promotions,
    })
}

pub fn render_organism(o: &Organism) -> String {
    let caps = o
        .capabilities
        .iter()
        .map(|c| {
            r::st(
                "OrganismCapability",
                vec![
                    ("key", r::s(&c.key)),
                    ("organ", r::s(&c.organ)),
                    (
                        "claimed",
                        en("GrowthState", c.claimed, GrowthState::variant),
                    ),
                    ("backend", r::opt(&c.backend)),
                    ("evaluations", r::strs(&c.evaluations)),
                ],
            )
        })
        .collect();
    let backends = o
        .backends
        .iter()
        .map(|b| {
            r::st(
                "Backend",
                vec![
                    ("key", r::s(&b.key)),
                    ("kind", en("BackendKind", b.kind, BackendKind::variant)),
                    ("node", r::s(&b.node)),
                    ("donor", r::opt(&b.donor)),
                    ("weight", r::s(&b.weight)),
                ],
            )
        })
        .collect();
    let promotions = o
        .promotions
        .iter()
        .map(|p| {
            r::st(
                "Promotion",
                vec![
                    ("key", r::s(&p.key)),
                    ("source_repository", r::s(&p.source_repository)),
                    ("source_node", r::s(&p.source_node)),
                    ("source_capability", r::opt(&p.source_capability)),
                    ("source_technology", r::opt(&p.source_technology)),
                    ("source_commit", r::s(&p.source_commit)),
                    ("source_evidence", r::strs(&p.source_evidence)),
                    ("destination_capability", r::s(&p.destination_capability)),
                    ("destination_node", r::opt(&p.destination_node)),
                    ("relation", en("EdgeKind", p.relation, EdgeKind::variant)),
                    (
                        "implementation",
                        en(
                            "PromotionImplementation",
                            p.implementation,
                            PromotionImplementation::variant,
                        ),
                    ),
                    (
                        "claimed",
                        en("PromotionState", p.claimed, PromotionState::variant),
                    ),
                    ("reason", r::s(&p.reason)),
                    ("transformation", r::s(&p.transformation)),
                ],
            )
        })
        .collect();
    let e = r::st(
        "Organism",
        vec![
            ("capabilities", r::list(caps)),
            ("backends", r::list(backends)),
            ("promotions", r::list(promotions)),
        ],
    );
    format!("{HEADER}{}", r::render(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Declaration {
        Declaration {
            repository: Repository {
                system: "chronica".into(),
                shard: "demo".into(),
                name: "Demo".into(),
                origin: "llgtrn/Demo".into(),
                nodes: vec![Node {
                    key: "core".into(),
                    kind: NodeKind::Kernel,
                    concept: Concept::Subsystem,
                    name: "Kernel".into(),
                    path: "core".into(),
                    canonical_path: "core".into(),
                    lifecycle: NodeLifecycle::Active,
                    provides: vec!["identity".into()],
                    requires: vec![],
                    reuses: vec![],
                    inputs: vec![],
                    outputs: vec![],
                    lineage: vec!["serde".into()],
                }],
                edges: vec![Edge {
                    from: "core".into(),
                    to: "other:core".into(),
                    kind: EdgeKind::DependsOn,
                    scope: Scope::Runtime,
                }],
            },
            donors: vec![Donor {
                key: "serde".into(),
                name: "serde".into(),
                origin: "https://github.com/serde-rs/serde".into(),
                license: "MIT OR Apache-2.0".into(),
                claimed: DonorState::Registered,
                exception: Some((ExceptionKind::Blocked, "needs a \"native\" derive".into())),
                packages: vec![Package {
                    ecosystem: Ecosystem::Cargo,
                    name: "serde".into(),
                }],
                source_paths: vec![],
                capabilities: vec![Capability {
                    key: "serialize".into(),
                    required: true,
                    spec: "tests/serialize.rs".into(),
                    replacement: Some("core".into()),
                    maps_to: Some("technology/hash.sha256".into()),
                    norl: NorlRelevance::NotRelevant("serialization is plumbing".into()),
                    proofs: vec![Proof {
                        kind: ProofKind::Parity,
                        locator: "tests/serialize.rs::agrees".into(),
                    }],
                }],
                cutover: None,
                provenance: vec!["legacy:.atlas/x.toml#serde".into()],
            }],
            migration: Migration {
                waves: vec![Wave {
                    key: "w1".into(),
                    status: WaveStatus::Planned,
                    nodes: vec!["core".into()],
                }],
                shims: vec![Shim {
                    key: "atlas".into(),
                    kind: ShimKind::LegacyInput,
                    path: ".atlas".into(),
                    serves: "demo".into(),
                    expires: (ExpiryKind::LegacyImported, ".atlas".into()),
                }],
            },
            technologies: vec![Technology {
                key: "hash.sha256".into(),
                name: "SHA-256".into(),
                kind: TechnologyKind::Algorithm,
                claimed: TechnologyLifecycle::Proven,
                purpose: "content identity".into(),
                implements: vec!["identity.digest".into()],
                node: "core".into(),
                sources: vec!["core/src/sha.rs".into()],
                invariants: vec!["FIPS 180-4 vectors".into()],
                proofs: vec![Proof {
                    kind: ProofKind::Regression,
                    locator: "core/src/sha.rs::vectors".into(),
                }],
                lineage: vec!["serde".into()],
                relations: vec![Relation {
                    kind: EdgeKind::Evolves,
                    target: "hash.sha256-v0".into(),
                }],
                norl: NorlRelevance::Feeds("memory.recall".into()),
                claims: vec![Improvement {
                    dimension: Dimension::DependencyCount,
                    baseline: "sha2".into(),
                    workload: "any".into(),
                    statement: "zero dependencies".into(),
                    evidence: vec![],
                }],
            }],
            organism: Organism {
                capabilities: vec![OrganismCapability {
                    key: "memory.recall".into(),
                    organ: "organ.memory".into(),
                    claimed: GrowthState::Exposed,
                    backend: Some("recall.rules".into()),
                    evaluations: vec!["eval.recall".into()],
                }],
                backends: vec![Backend {
                    key: "recall.rules".into(),
                    kind: BackendKind::Deterministic,
                    node: "core".into(),
                    donor: None,
                    weight: String::new(),
                }],
                promotions: vec![Promotion {
                    key: "recall.from-chronica".into(),
                    source_repository: "chronica".into(),
                    source_node: "substrate.memory".into(),
                    source_capability: Some("memory.recall".into()),
                    source_technology: Some("index.btree".into()),
                    source_commit: "0aaf14255".into(),
                    source_evidence: vec!["substrate/memory/src/lib.rs::recalls".into()],
                    destination_capability: "memory.recall".into(),
                    destination_node: None,
                    relation: EdgeKind::Feeds,
                    implementation: PromotionImplementation::IndependentImplementation,
                    claimed: PromotionState::PromotionDesigned,
                    reason: "Norl must recall".into(),
                    transformation: "re-derived over Norl's episodic store".into(),
                }],
            },
        }
    }

    #[test]
    fn render_parse_roundtrip_is_exact() {
        let d = sample();
        let t1 = render_repository(&d.repository);
        let t2 = render_donors(&d.donors);
        let t3 = render_migration(&d.migration);
        assert_eq!(parse_repository(&t1).ok().unwrap(), d.repository);
        assert_eq!(parse_donors(&t2).ok().unwrap(), d.donors);
        assert_eq!(parse_migration(&t3).ok().unwrap(), d.migration);
        let t4 = render_technologies(&d.technologies);
        assert_eq!(parse_technologies(&t4).ok().unwrap(), d.technologies);
        let t5 = render_organism(&d.organism);
        assert_eq!(parse_organism(&t5).ok().unwrap(), d.organism);
        assert_eq!(render_organism(&parse_organism(&t5).ok().unwrap()), t5);
        // Canonical rendering is a fixed point.
        assert_eq!(render_donors(&parse_donors(&t2).ok().unwrap()), t2);
    }

    #[test]
    fn unknown_and_missing_fields_are_rejected() {
        let bad = "Migration { waves: &[], shims: &[], extra: 1 }";
        assert!(parse_migration(bad)
            .err()
            .unwrap()
            .message
            .contains("unknown field"));
        let missing = "Migration { waves: &[] }";
        assert!(parse_migration(missing)
            .err()
            .unwrap()
            .message
            .contains("missing field"));
        let wrong = "&[Donor { key: \"x\", name: \"x\", origin: \"\", license: \"\", claimed: DonorState::Absorbed, exception: Exception::None, packages: &[], source_paths: &[], capabilities: &[], cutover: None, provenance: &[] }]";
        assert!(parse_donors(wrong)
            .err()
            .unwrap()
            .message
            .contains("not a DonorState"));
    }
}
