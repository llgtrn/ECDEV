//! The existing read-only proposal commands accept an explicitly pinned current capture.
mod common;

use ecdev_governance::digest::content_digest;
use ecdev_governance::formats::json;
use ecdev_governance::migration::source;

#[test]
fn current_capture_cli_binds_the_caller_digest_without_installing_authority() {
    let root = ecdev_governance::default_root();
    let capture = source::capture_committed(&root).unwrap();
    let raw = capture.render().unwrap();
    let hash = content_digest(raw.as_bytes())[7..].to_string();
    let r = common::Repo::new("current-mapping-cli");
    r.write("capture.json", &raw);
    let from = r.root.join("capture.json").display().to_string();
    let (code, proposal) = r.cli(&[
        "migrate",
        "propose-mapping",
        "--from",
        &from,
        "--expect-sha256",
        &hash,
    ]);
    assert_eq!(code, 0, "{proposal}");
    let value = json::parse_unique(&proposal).unwrap();
    let source_nodes = json::parse_unique(&raw).unwrap();
    assert_eq!(
        value.items_of("node_mappings").len(),
        source_nodes.at("source_graph.nodes").unwrap().items().len()
    );
    assert_eq!(
        value.get("target_custody"),
        Some(&ecdev_governance::formats::Value::Null)
    );
    r.write("proposal.json", &proposal);
    let proposed = r.root.join("proposal.json").display().to_string();
    let (code, inspected) = r.cli(&[
        "migrate",
        "inspect-proposal",
        "--from",
        &proposed,
        "--capture",
        &from,
        "--expect-sha256",
        &hash.to_ascii_uppercase(),
    ]);
    assert_eq!(code, 0, "{inspected}");
    assert_eq!(
        json::parse_unique(&inspected).unwrap().str("status"),
        Some("VALID_PENDING_PROPOSAL")
    );
    for pin in ["0".repeat(64), format!("sha256:{hash}"), "z".repeat(64)] {
        assert_eq!(
            r.cli(&[
                "migrate",
                "propose-mapping",
                "--from",
                &from,
                "--expect-sha256",
                &pin
            ])
            .0,
            2
        );
    }
    assert_eq!(source::capture_committed(&root).unwrap(), capture);
    // Exact directory coverage proves that no authority, control state, proof or build output
    // was installed under any name by either read-only command.
    let mut entries: Vec<_> = std::fs::read_dir(&r.root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    entries.sort();
    assert_eq!(entries, ["capture.json", "proposal.json"]);
    assert_eq!(
        std::fs::read_to_string(r.root.join("capture.json")).unwrap(),
        raw
    );
}
