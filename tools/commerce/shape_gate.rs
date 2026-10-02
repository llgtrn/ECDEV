//! Read-only gate using the canonical Ynventa assessor, role table and ownership index.
//! Build after `cargo build --manifest-path .ynventa/Cargo.toml --lib`:
//! rustc --edition=2021 tools/commerce/shape_gate.rs --extern ynventa=.ynventa/target/debug/libynventa.rlib -L dependency=.ynventa/target/debug/deps -o target/shape-gate
use ynventa::{
    donors::NodeIndex,
    formats::json::Json,
    repository::{role_of_path, root_file_allowed},
    schema::{is_physical, NodeLifecycle},
};
fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let a = ynventa::assess(std::path::Path::new(&root)).expect("canonical assessment");
    let index = NodeIndex::new(&a.declaration);
    let mut owned = 0u64;
    let mut unowned = 0u64;
    let mut inventory = Vec::new();
    let mut forbidden = Vec::new();
    for f in &a.files.paths {
        // Donor snapshot contents are not tracked production paths. Census records are material.
        if f.split('/').any(|s| s == "crate" || s == "crates") {
            forbidden.push(f.clone());
        }
        let owner = index.owner(f);
        if [
            ".rs", ".ts", ".tsx", ".js", ".mjs", ".py", ".c", ".cc", ".cpp", ".go",
        ]
        .iter()
        .any(|s| f.ends_with(s))
        {
            if owner.is_some() {
                owned += 1;
            } else {
                unowned += 1;
            }
        }
        let node = owner.and_then(|k| a.declaration.node(k));
        inventory.push(
            Json::obj()
                .with("path", f.as_str())
                .with("owner", owner.unwrap_or("INFRASTRUCTURE"))
                .with(
                    "kind",
                    node.map(|n| n.kind.wire()).unwrap_or("INFRASTRUCTURE"),
                )
                .with(
                    "capabilities",
                    node.map(|n| n.provides.clone()).unwrap_or_default(),
                )
                .with(
                    "dependencies",
                    a.declaration
                        .repository
                        .edges
                        .iter()
                        .filter(|e| Some(e.from.as_str()) == owner)
                        .map(|e| e.to.clone())
                        .collect::<Vec<_>>(),
                ),
        );
    }
    let mut duplicate = 0u64;
    let nodes: Vec<_> = a
        .declaration
        .repository
        .nodes
        .iter()
        .filter(|n| is_physical(n.kind) && n.lifecycle != NodeLifecycle::Retired)
        .collect();
    for (i, n) in nodes.iter().enumerate() {
        for m in nodes.iter().skip(i + 1) {
            if n.path == m.path {
                duplicate += 1;
            }
        }
    }
    let roots = a.files.root_entries();
    let invalid = roots
        .iter()
        .filter(|p| {
            if a.files.is_dir(p) {
                role_of_path(p).is_none()
            } else {
                !root_file_allowed(p)
            }
        })
        .count() as u64;
    let mismatches = a
        .shape
        .findings
        .iter()
        .filter(|f| f.code == "NONCANONICAL_TARGET" || f.code == "LEGACY_PLACEMENT")
        .count() as u64;
    let ok = a.shape.units == a.shape.conformant
        && unowned == 0
        && duplicate == 0
        && forbidden.is_empty();
    let report = Json::obj()
        .with("status", if ok { "PASS" } else { "FAIL" })
        .with(
            "authority",
            ".ynventa/src/repository/{mod,shape,files}.rs; .ynventa/src/donors/mod.rs",
        )
        .with("canonical_nodes", a.shape.nodes_total)
        .with("conformant_nodes", a.shape.nodes_conformant)
        .with(
            "nonconformant_nodes",
            a.shape.nodes_total - a.shape.nodes_conformant,
        )
        .with("owned_source_files", owned)
        .with("unowned_source_files", unowned)
        .with("valid_top_level_roots", roots.len() as u64 - invalid)
        .with("invalid_top_level_roots", invalid)
        .with("duplicate_ownership", duplicate)
        .with("path_kind_mismatches", mismatches)
        .with("shape_units", a.shape.units)
        .with("conformant_shape_units", a.shape.conformant)
        .with(
            "shape_conformance_bps",
            if a.shape.units == 0 {
                0
            } else {
                a.shape.conformant * 10000 / a.shape.units
            },
        )
        .with("forbidden_architecture_paths", forbidden)
        .with(
            "findings",
            Json::Array(a.shape.findings.iter().map(|f| f.to_json()).collect()),
        )
        .with("file_classification", Json::Array(inventory));
    println!("{}", report.render().trim_end());
    if !ok {
        std::process::exit(1);
    }
}
