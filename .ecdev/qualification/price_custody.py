#!/usr/bin/env python3
"""Preserve a partial four-node source custody witness and blocked cutover prerequisites.

This is a lossless source witness, not a positive enrollment or authority grammar.
All Git reads name immutable commits; no checkout, runtime, or target call is made.
"""

import argparse
import copy
import hashlib
from pathlib import Path
import stat
import sys

import price_candidate as candidate
import price_caller_readiness as caller
import price_cohort as cohort
import price_manifest as manifest
import run_candidate as immutable


ROOT = Path(__file__).absolute().parents[2]
HISTORICAL_COMMIT = "db92bf27d31409704fccd44defe342a1b2f22823"
HISTORICAL_TREE = "ec88cd6c52fadf3c638bed0eb7b790096eab8617"
CURRENT_COMMIT = "60b43d97a0013fea9497ae91215027ba1a9de43e"
CURRENT_TREE = "48ee94266e011ad4b06476b0ab4c722689b6f936"
HISTORICAL_CALLER_SHA256 = "d7f005b5a5fdc637346cf90700d23f0558704d0f30cfea171a451bfaa6fa05e4"
REPORT_PATH = "research/commerce/governance/price-web-caller-acceptance-v1.json"
REPORT_SHA256 = "4a3c8b319133463772fae4a169a1d3e3ef68381404ed6d9debf6dcbdf3c86186"
READINESS_PATH = "research/commerce/governance/price-web-caller-readiness-v1.json"
READINESS_SHA256 = "7b2ca2b6d93cb4effe2b8c37808b6b087b86c4213516532299a22f0daa96091d"
SELECTED = {"donor_capability": 306, "source_caller": 503,
            "source_capability": 513, "source_technology": 723}
MATERIAL_PATHS = (
    "adapter/web/src/lib.rs", "adapter/web/src/price.rs", "adapter/web/data/price-currency.json",
    "adapter/web/tests/fixtures/price-parser-LICENSE.txt",
    "research/commerce/donors/census/scrapinghub--price-parser/license.json",
    ".ecdev/declared/donors.rs", ".ecdev/declared/repository.rs", ".ecdev/declared/technologies.rs",
)
BOUNDARY_PATHS = ("Cargo.toml", "adapter/web/Cargo.toml", "domain/commerce/Cargo.toml")
FORMAT = "ecdev-price-selected-source-custody-v1"
SCOPE = "FOUR_EXISTING_SOURCE_NODES; PARTIAL_MATERIAL_AND_HISTORICAL_PROOF_CUSTODY_ONLY"
INPUTS = {**cohort.ARTIFACTS,
          "cohort": (caller.COHORT_PATH, caller.COHORT_SHA256),
          "corpus": (manifest.CORPUS_PATH, manifest.CORPUS_SHA256),
          "web_report": (REPORT_PATH, REPORT_SHA256),
          "web_readiness": (READINESS_PATH, READINESS_SHA256),
          "target_owner_contract": (candidate.TARGET_CONTRACT_PATH, candidate.TARGET_CONTRACT_SHA256)}


def same(actual, expected, message):
    if candidate.canonical(actual) != candidate.canonical(expected):
        raise ValueError(message)


def blob_id(raw):
    return hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest()


def tree_entries(root, commit, tree):
    """Use the restricted, replacement-free, no-fetch Git helper; never resolve HEAD."""
    if immutable.git(root, "cat-file", "-t", commit).strip() != b"commit":
        raise ValueError("fixed source commit is unavailable")
    if immutable.git(root, "rev-parse", commit + "^{tree}").decode().strip() != tree:
        raise ValueError("fixed source tree differs")
    return immutable.parse_tree(immutable.git(root, "ls-tree", "-r", "-z", "--full-tree", commit))


def committed_file(root, entries, path, expected_member=None, include_hex=True, worktree=False):
    path = immutable.safe_relative(path)
    if path not in entries:
        raise ValueError("selected committed material is missing: " + path)
    mode, blob = entries[path]
    size = int(immutable.git(root, "cat-file", "-s", blob))
    if not 0 <= size <= candidate.MAX_INPUT:
        raise ValueError("selected committed material exceeds the regular-file budget")
    raw = immutable.git(root, "cat-file", "blob", blob)
    if len(raw) != size or blob_id(raw) != blob:
        raise ValueError("committed selected blob length or identity differs")
    member = {"path": path, "mode": mode, "blob": blob, "sha256": cohort.digest(raw), "bytes": size}
    if expected_member is not None:
        same(member, expected_member, "historical selected material differs from frozen membership")
    if worktree:
        actual = candidate.regular(root / path)
        actual_mode = "100755" if (root / path).stat().st_mode & 0o111 else "100644"
        if actual != raw or actual_mode != mode:
            raise ValueError("current selected bytes or mode differ from fixed current commit: " + path)
    return {**member, **({"hex": raw.hex()} if include_hex else {})}


def load_inputs(root, expected_report_sha256, expected_readiness_sha256):
    frozen = {role: candidate.regular(root / path) for role, (path, _) in INPUTS.items()}
    cohort.require_pin(frozen["web_report"], expected_report_sha256)
    cohort.require_pin(frozen["web_readiness"], expected_readiness_sha256)
    for role, (_, pin) in INPUTS.items():
        cohort.require_pin(frozen[role], pin)
    # Independently regenerate both the frozen source cohort and the actual-caller request data.
    report, selected, _ = caller.inspect_report(frozen["web_report"], expected_report_sha256, root)
    regenerated = caller.build_readiness(frozen["web_report"], expected_report_sha256, root)
    same(candidate.strict_json(frozen["web_readiness"]), regenerated, "fixed Web readiness differs")
    same(candidate.strict_json(frozen["cohort"]), selected, "fixed selected cohort differs")
    return frozen, report, selected, regenerated


def source_status():
    return {"source_authority": "SOURCE_ONLY", "canonical_enrollment": "BLOCKED",
            "canonical_assignment": None, "accepted_owner": None, "authority": None,
            "authority_grants": [], "current_rights": None, "copying_authorization": "UNKNOWN",
            "target_exclusive_custody": None, "Host_admission": None, "world_admission": None,
            "admission": "PENDING", "source_adapter_cutover": False, "source_retirement": False,
            "target_execution_performed": False, "target_acceptance_established": False,
            "enrollment_performed": False, "exclusive_execution_custody": "NOT_ESTABLISHED"}


def prerequisite_contract(report, readiness, owner):
    """Concrete source vectors for a future target-owned, authenticated execution assay."""
    obligations = [
        {"key": "target_execution", "owner": "target caller/runtime owner", "evidence": None,
         "requires": ["Execute all 99 exact vectors through the real maintained caller and governed runtime",
                      "Bind exact target commit/tree, capability, implementation, caller, binary and dependency digests",
                      "Compare original four fields and nullable finite float bits with the executed settled result"]},
        {"key": "exclusive_custody", "owner": "target custody and source owner", "evidence": None,
         "requires": ["Accept the source-selected material and retained lineage under a published custody consumer",
                      "Supply genuine exclusive execution custody evidence; source copy or digest is insufficient",
                      "Keep historical source proofs distinct from current source and target attestations"]},
        {"key": "authority", "owner": "target authority/operator", "evidence": None,
         "requires": ["Authenticate requester and independently assess the exact implementation",
                      "Bind current resource.work, lease/release, purpose, restrictions, expiry and revocation",
                      "Bind one provisional cpu_units allocation and separate current resource.observe admission",
                      "Preserve full IR, original payload, selector presence, provider audience and complete EffectRef joins"]},
        {"key": "unknown_effect", "owner": "target effect/evidence owner", "evidence": None,
         "requires": ["Demonstrate pending EffectUnknown evidence stays unchanged without resend, recompute or refund",
                      "Demonstrate typed known unavailable is a settled false result, not fabricated source success",
                      "Read and strictly decode the canonical receipt through the maintained authorized observation path"]},
        {"key": "rights_and_enrollment", "owner": "source and target rights/enrollment owners", "evidence": None,
         "requires": ["Publish genuine current copying/redistribution and use decisions without deriving rights from a license claim",
                      "Accept source identity mapping, shared protocol compatibility and enrollment with a real authority validator",
                      "Retain original IDs, ECG history, source metadata, all incident edges and unresolved obligations"]},
        {"key": "production_cutover", "owner": "actual source production caller owner", "evidence": None,
         "requires": ["Install and qualify the actual production caller route only after every preceding gate is satisfied",
                      "Keep 2611 Web-shaped unmapped cases and 227 non-Web-shaped cases outside the selected route",
                      "Keep all seven API-only exclusions separate and preserve the full 106/2831/2937 API partition",
                      "Preserve unknown observed Money, raw tokens and bytes; derived output grants no ISO identity or money authority"]},
    ]
    vectors = []
    for case, proposed in zip(report["cases"], readiness["proposed_requests"]):
        if case["index"] != proposed["source_case_index"]:
            raise ValueError("source Web vectors and readiness indices differ")
        vectors.append({"index": case["index"], "name": case["name"],
                        "source_request": copy.deepcopy(case["source_request"]),
                        "source_Option_presence": copy.deepcopy(case["source_request_presence"]),
                        "payload_utf8_hex": proposed["payload_utf8_hex"],
                        "payload_sha256": proposed["payload_sha256"],
                        "text_selector_facts": copy.deepcopy(proposed["text_selector_facts"]),
                        "input_is_none_fact_present": False,
                        "expected_source_four_fields": copy.deepcopy(case["interpretation"]["result"]),
                        "expected_source_float_bits": case["interpretation"]["amount_float_bits"],
                        "source_observed": copy.deepcopy(case["source_observed"]),
                        "raw_json_ld": copy.deepcopy(case["raw_json_ld"]),
                        "target_execution": "NOT_RUN", "target_result": None, "canonical_receipt": None,
                        "full_IR_digest": None, "EffectRef": None, "current_rights": None,
                        "used_for_observed_price": False, "currency_token_is_iso_identity": False})
    return {"format": "ecdev-price-parity-cutover-prerequisites-v1",
            "classification": "SOURCE_VECTOR_AND_TARGET_OWNED_OBLIGATION_CONTRACT; NO_TARGET_EXECUTION",
            "source_report_sha256": REPORT_SHA256, "source_readiness_sha256": READINESS_SHA256,
            "frozen_cohort_sha256": caller.COHORT_SHA256,
            "counts": copy.deepcopy(report["counts"]),
            "selected_web_indices": [row["index"] for row in report["cases"]],
            "cohort_distinction": "These 99 are 94 prior plus 5 additions; historical old99 remains separate",
            "existing_target_reference": copy.deepcopy(owner["target"]),
            "target_contract_reference": {"path": candidate.TARGET_CONTRACT_PATH,
                                          "sha256": candidate.TARGET_CONTRACT_SHA256,
                                          "role": "EXISTING_PUBLISHED_REFERENCE_ONLY; NO_NEW_ATTESTATION"},
            "target_request_result_admission_contract": copy.deepcopy(owner["exact_input_output_and_admission"]),
            "target_reference_status_dimensions": copy.deepcopy(owner["status_dimensions"]),
            "vectors": vectors, "API_only_exclusions": copy.deepcopy(report["excluded_cases"]),
            "source_boundary_records": copy.deepcopy(report["negative_cases"]),
            "boundary_requirement": "All 20 source boundary records remain exact negative/preservation evidence; target assay must separately prove known unavailability and unknown-effect behavior",
            "obligations": obligations, "target_execution": "NOT_RUN", "target_authority_validator": None,
            "positive_target_qualification_supported": False, "status": source_status()}


def build_witness(root=ROOT, *, expected_report_sha256, expected_readiness_sha256):
    root = candidate.physical(root)
    if not stat.S_ISDIR(root.stat().st_mode):
        raise ValueError("source root must be a physical directory")
    frozen, report, selected, readiness = load_inputs(root, expected_report_sha256, expected_readiness_sha256)
    capture = candidate.strict_json(frozen["capture"])
    proposal = candidate.strict_json(frozen["proposal"])
    membership = candidate.strict_json(frozen["source_membership"])
    owner = candidate.strict_json(frozen["target_owner_contract"])
    historical = tree_entries(root, HISTORICAL_COMMIT, HISTORICAL_TREE)
    current = tree_entries(root, CURRENT_COMMIT, CURRENT_TREE)
    same((membership["source_commit"], membership["source_tree"]),
         (HISTORICAL_COMMIT, HISTORICAL_TREE), "frozen historical membership rebind")
    members = {row["path"]: row for row in membership["inputs"]}
    graph = capture["source_graph"]
    bindings = selected["source_bindings"]
    nodes = []
    for role, index in SELECTED.items():
        node = graph["nodes"][index]
        mapping = proposal["node_mappings"][index]
        binding = bindings[role]
        if (node["id"] != binding["source_id"] or mapping["source_id"] != node["id"]
                or binding["capture_locator"] != f"/source_graph/nodes/{index}"
                or mapping["source_metadata"] != node or mapping["proposed_target"] is not None):
            raise ValueError("selected original node or null source mapping differs")
        nodes.append({"role": role, "capture_index": index, "binding": copy.deepcopy(binding),
                      "original_node": copy.deepcopy(node), "original_mapping": copy.deepcopy(mapping)})
    ids = {row["original_node"]["id"] for row in nodes}
    incident = [{"capture_locator": f"/source_graph/edges/{i}", "original_edge": copy.deepcopy(edge)}
                for i, edge in enumerate(graph["edges"]) if edge["from"] in ids or edge["to"] in ids]
    adjacent = {row["original_edge"][field] for row in incident for field in ("from", "to")}
    context = [{"capture_locator": f"/source_graph/nodes/{i}", "original_node": copy.deepcopy(node)}
               for i, node in enumerate(graph["nodes"]) if node["id"] in adjacent]
    if len(nodes) != 4 or len(incident) != 72 or len(context) != 69:
        raise ValueError("selected four-node incidence boundary differs")
    proof_locators = graph["nodes"][SELECTED["source_technology"]]["evidence"]
    captured_files = {row["path"]: (i, row) for i, row in enumerate(capture["files"])}
    proof_paths = []
    proof_links = {locator: [] for locator in proof_locators}
    for path, (_, row) in captured_files.items():
        if path.startswith(".ecdev/evidence/"):
            raw = bytes.fromhex(row["hex"])
            matching = [locator for locator in proof_locators if locator.encode() in raw]
            if matching:
                if not raw.startswith(b"ECG\x01"):
                    raise ValueError("selected proof record is not retained ECG version 1")
                proof_paths.append(path)
                for locator in matching:
                    proof_links[locator].append(path)
    if len(proof_locators) != 10 or len(proof_paths) != 62 or not all(proof_links.values()):
        raise ValueError("historical selected proof material boundary differs")
    history_paths = sorted(path for path in captured_files if path.startswith(".ecdev/history/"))
    if len(history_paths) != 1:
        raise ValueError("retained history payload boundary differs")
    material_paths = sorted({*MATERIAL_PATHS, *proof_paths, *history_paths})
    material = []
    for path in material_paths:
        old = committed_file(root, historical, path, members[path])
        new = committed_file(root, current, path, worktree=True)
        capture_reference = None
        if path in captured_files:
            index, captured = captured_files[path]
            if old["hex"] != captured["hex"] or old["sha256"] != captured["sha256"]:
                raise ValueError("captured selected payload bytes differ from historical Git member")
            capture_reference = {"artifact": "capture", "locator": f"/files/{index}",
                                 "sha256": captured["sha256"], "raw_bytes_preserved": True}
        material.append({"path": path, "historical": old, "fixed_current": new,
                         "changed_since_capture": old != new,
                         "capture_record": capture_reference,
                         "proof_use": "HISTORICAL_RECORD_BYTES_ONLY; NOT_A_CURRENT_TEST_OR_TARGET_ATTESTATION"
                         if path in proof_paths else None})
    lib = next(row for row in material if row["path"] == caller.CALLER_PATH)
    if lib["historical"]["sha256"] != HISTORICAL_CALLER_SHA256 or lib["fixed_current"]["sha256"] != caller.CALLER_SHA256:
        raise ValueError("historical/current caller separation differs")
    price = next(row for row in material if row["path"] == caller.PRICE_PATH)
    if price["historical"]["sha256"] != caller.PRICE_SHA256 or price["changed_since_capture"]:
        raise ValueError("price parser unexpectedly differs from frozen historical parser")
    external_paths = sorted({*graph["nodes"][SELECTED["source_technology"]]["inputs"], *BOUNDARY_PATHS} - set(material_paths))
    external = [{"path": path, "historical": committed_file(root, historical, path, members[path], False),
                 "fixed_current": committed_file(root, current, path, include_hex=False, worktree=True),
                 "bytes_embedded": False, "retrieval": "EXACT_IMMUTABLE_GIT_BLOB_OR_PINNED_REGULAR_SOURCE_FILE"}
                for path in external_paths]
    corpus = next(row for row in external if row["path"] == manifest.CORPUS_PATH)
    if (corpus["historical"]["sha256"] != manifest.CORPUS_SHA256
            or corpus["fixed_current"]["sha256"] != manifest.CORPUS_SHA256):
        raise ValueError("corpus historical/current external reference differs")
    artifacts = {}
    for role, (path, pin) in INPUTS.items():
        member = committed_file(root, current, path, include_hex=False, worktree=True)
        if member["sha256"] != pin or member["bytes"] != len(frozen[role]):
            raise ValueError("fixed current artifact member differs")
        artifacts[role] = member
    donor = next(row for row in proposal["source_semantics"]["donors"]
                 if row["source_alias"] == "scrapinghub--price-parser")
    claims = [row for row in proposal["rights_claims"] if row["source_id"] == donor["source_id"]]
    if len(claims) != 1:
        raise ValueError("original donor rights claim boundary differs")
    semantics = proposal["source_semantics"]
    witness = {"format": FORMAT, "scope": SCOPE, "artifacts": artifacts,
               "historical_source": {"repository": capture["source_repository"], "source_id": capture["source_id"],
                                     "commit": HISTORICAL_COMMIT, "tree": HISTORICAL_TREE,
                                     "capture_binding": copy.deepcopy(proposal["capture_binding"]),
                                     "membership_flags": {k: copy.deepcopy(v) for k, v in membership.items() if k != "inputs"}},
               "fixed_current_source": {"repository": capture["source_repository"], "commit": CURRENT_COMMIT,
                                        "tree": CURRENT_TREE, "meaning": "FIXED_SOURCE_BYTES; NOT_FUTURE_HEAD_OR_TARGET_CUSTODY"},
               "selected_nodes": nodes, "incident_edges": incident,
               "adjacent_context_nodes": context,
               "context_scope": "69 original incident endpoints are context only; selected ownership scope stays exactly four nodes",
               "raw_material": material, "external_material_references": external,
               "material_boundary": {
                   "classification": "PARTIAL_SELECTED_SOURCE_MATERIAL; NOT_COMPLETE_CRATE_OR_RUNTIME_CLOSURE",
                   "embedded_paths": material_paths, "historical_proof_record_paths": sorted(proof_paths),
                   "historical_proof_locator_records": proof_links,
                   "selection": "Eight explicit runtime/license/declaration files, every historical ECG containing one of technology723's ten proof locators, and the single captured history ECG",
                   "external_only_paths": external_paths,
                   "source_parser_runtime_dependency": "price.rs includes price-currency.json and uses the core JSON API and standard library",
                   "source_public_caller_dependency": "lib.rs also depends on its HTML, currency, commerce annotation, microdata and page extraction modules plus core JSON, URL and domain APIs",
                   "unclosed_dependencies": ["Other adapter/web source modules", "domain/commerce implementation and transitive dependencies",
                                             "native HTTP/TLS/network runtime", "complete build/test fixture closure", "target binary/runtime dependencies"],
                   "proof_limits": "All original selected-node metadata is retained; the caller's unrelated proofs and transitive crate closure are not asserted by this selected-material witness"},
               "source_lineage_and_rights": {
                   "original_donor_semantics": copy.deepcopy(donor), "original_rights_claims": copy.deepcopy(claims),
                   "source_history": copy.deepcopy(semantics["history"]),
                   "record_counts": {key: semantics[key] for key in ("active_evidence_records", "retired_evidence_records")},
                   "source_semantics_flags": {key: copy.deepcopy(semantics[key]) for key in
                                              ("binding", "commit_membership", "canonical_enrollment", "canonical_identity_mapping",
                                               "norl_relevance_mapping", "recorded_schema_claim")},
                   "proof_reinterpretation_performed": False, "history_completeness": membership["history_completeness"],
                   "rights_authorization": membership["rights_authorization"], "current_rights": None,
                   "copying_authorization": "UNKNOWN", "original_source_ids_and_ECG_bytes_preserved": True},
               "caller_version_separation": {"path": caller.CALLER_PATH,
                                             "historical_sha256": lib["historical"]["sha256"],
                                             "fixed_current_sha256": lib["fixed_current"]["sha256"],
                                             "historical_proof_reused_as_current_attestation": False,
                                             "current_source_evidence_sha256": REPORT_SHA256,
                                             "current_source_readiness_sha256": READINESS_SHA256,
                                             "price_parser_unchanged": True, "corpus_unchanged": True},
               "parity_cutover_prerequisites": prerequisite_contract(report, readiness, owner),
               "preservation": {"source_proposal_rewritten": False, "source_capture_rewritten": False,
                                "identity_remap_performed": False, "target_calls_performed": False,
                                "source_guard_modified": False, "runtime_data_migrated": False},
               "status": source_status()}
    # Re-read every input and selected worktree byte after construction; the immutable Git objects
    # bind the two source versions even when publication HEAD subsequently advances.
    for role, (path, _) in INPUTS.items():
        if candidate.regular(root / path) != frozen[role]:
            raise ValueError("frozen witness input changed during construction")
    for row in [*material, *external]:
        member = row["fixed_current"]
        raw = candidate.regular(root / row["path"])
        mode = "100755" if (root / row["path"]).stat().st_mode & 0o111 else "100644"
        if cohort.digest(raw) != member["sha256"] or len(raw) != member["bytes"] or mode != member["mode"]:
            raise ValueError("selected current material changed during construction")
    return witness


def inspect_witness(raw, expected_witness_sha256, root=ROOT, *, expected_report_sha256,
                    expected_readiness_sha256):
    cohort.require_pin(raw, expected_witness_sha256)
    expected = build_witness(root, expected_report_sha256=expected_report_sha256,
                             expected_readiness_sha256=expected_readiness_sha256)
    same(candidate.strict_json(raw), expected,
         "custody witness bytes, identities, edges, proof lineage, rights or source versions changed")
    return {"format": "ecdev-price-selected-source-custody-inspection-v1",
            "status": "VALID_SOURCE_WITNESS; CUTOVER_BLOCKED", "witness_sha256": cohort.digest(raw),
            "selected_nodes": 4, "incident_edges": 72, "context_nodes": 69,
            "embedded_material_files": len(expected["raw_material"]),
            "source_report_sha256": REPORT_SHA256, "target_execution": "NOT_RUN",
            "target_exclusive_custody": None, "Host_admission": None,
            "current_rights": None, "canonical_enrollment": "BLOCKED", "source_adapter_cutover": False}


def cutover_readiness(raw, expected_witness_sha256, root=ROOT, *, expected_report_sha256,
                      expected_readiness_sha256, target_evidence=None):
    inspection = inspect_witness(raw, expected_witness_sha256, root,
                                 expected_report_sha256=expected_report_sha256,
                                 expected_readiness_sha256=expected_readiness_sha256)
    if target_evidence is not None:
        raise ValueError("target evidence cannot be accepted: a genuine target authority validator is not implemented")
    contract = candidate.strict_json(raw)["parity_cutover_prerequisites"]
    return {"format": "ecdev-price-cutover-readiness-v1", "status": "CUTOVER_BLOCKED",
            "source_witness_inspection": inspection, "missing_target_evidence": [
                obligation["key"] for obligation in contract["obligations"]],
            "target_owned_obligations": copy.deepcopy(contract["obligations"]),
            "target_execution": "NOT_RUN", "target_exclusive_custody": None, "Host_admission": None,
            "current_rights": None, "canonical_enrollment": "BLOCKED", "source_adapter_cutover": False,
            "positive_target_qualification_supported": False, "target_authority_validator": None}


def inspect_source_package(raw, expected_package_sha256, *, expected_source_commit,
                           expected_source_tree, expected_rights_sha256):
    """Consume pinned original custody evidence offline; leave every target gate open.

    Package membership does not regenerate historical proofs or establish current
    execution custody. No source checkout, Git operation or runtime is consulted.
    """
    import price_source_package as package

    inspection, evidence = package.inspect_package_evidence(
        raw, expected_package_sha256, expected_source_commit=expected_source_commit,
        expected_source_tree=expected_source_tree, expected_rights_sha256=expected_rights_sha256)
    witness = candidate.strict_json(evidence[package.WITNESS_PATH])
    contract = candidate.strict_json(evidence[package.PREREQUISITE_PATH])
    rights = candidate.strict_json(evidence[package.RIGHTS_PATH])
    same(witness["parity_cutover_prerequisites"], contract,
         "original witness and prerequisite contract differ")
    same({row["role"]: row["binding"] for row in witness["selected_nodes"]},
         rights["selected_source_bindings"], "original source bindings differ from published rights")
    same({row["role"]: row["capture_index"] for row in witness["selected_nodes"]},
         SELECTED, "original four-node selection differs")
    same(witness["status"], source_status(), "original witness source status changed")
    same(contract["status"], source_status(), "original prerequisite source status changed")
    obligations = contract["obligations"]
    if ([row["key"] for row in obligations] != ["target_execution", "exclusive_custody", "authority",
                                              "unknown_effect", "rights_and_enrollment", "production_cutover"]
            or any(row["evidence"] is not None or not row["requires"] for row in obligations)
            or contract["target_execution"] != "NOT_RUN"
            or contract["target_authority_validator"] is not None
            or contract["positive_target_qualification_supported"] is not False):
        raise ValueError("original target obligations must remain open without evidence or authority")
    return {"format": "ecdev-price-cutover-readiness-v1", "status": "CUTOVER_BLOCKED",
            "source_package_inspection": inspection,
            "verification_scope": "PORTABLE_MEMBERSHIP_AND_ORIGINAL_PINNED_SOURCE_EVIDENCE_ONLY",
            "source_checkout_consulted": False, "source_witness_regenerated": False,
            "original_evidence": {path: {"sha256": cohort.digest(data), "bytes": len(data)}
                                  for path, data in evidence.items()},
            "historical_source": copy.deepcopy(witness["historical_source"]),
            "fixed_current_source": copy.deepcopy(witness["fixed_current_source"]),
            "caller_version_separation": copy.deepcopy(witness["caller_version_separation"]),
            "selected_source_bindings": copy.deepcopy(rights["selected_source_bindings"]),
            "selected_nodes": len(witness["selected_nodes"]),
            "incident_edges": len(witness["incident_edges"]),
            "context_nodes": len(witness["adjacent_context_nodes"]),
            "context_nodes_adopted": 0, "all828_admitted_mappings": 0,
            "production_obligations_closed": 0,
            "missing_target_evidence": [row["key"] for row in obligations],
            "target_owned_obligations": copy.deepcopy(obligations),
            "parity_cutover_prerequisites": copy.deepcopy(contract),
            "target_execution": "NOT_RUN", "target_exclusive_custody": None, "Host_admission": None,
            "current_rights": None, "canonical_enrollment": "BLOCKED", "source_adapter_cutover": False,
            "positive_target_qualification_supported": False, "target_authority_validator": None,
            "source_status": source_status()}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT,
                        help="source checkout for legacy modes; ignored for offline package inspection")
    parser.add_argument("--expect-report-sha256")
    parser.add_argument("--expect-readiness-sha256")
    parser.add_argument("--inspect-witness", type=Path)
    parser.add_argument("--expect-witness-sha256")
    operation = parser.add_mutually_exclusive_group()
    operation.add_argument("--contract", action="store_true")
    operation.add_argument("--check-cutover", action="store_true")
    operation.add_argument("--inspect-source-package", type=Path)
    parser.add_argument("--expect-package-sha256")
    parser.add_argument("--expect-source-commit")
    parser.add_argument("--expect-source-tree")
    parser.add_argument("--expect-rights-sha256")
    args = parser.parse_args()
    package_pins = (args.expect_package_sha256, args.expect_source_commit,
                    args.expect_source_tree, args.expect_rights_sha256)
    if args.inspect_source_package is not None:
        if any(pin is None for pin in package_pins):
            parser.error("package inspection requires independent package SHA-256, source commit, source tree and rights SHA-256")
        if any(value is not None for value in (args.inspect_witness, args.expect_witness_sha256,
                                               args.expect_report_sha256, args.expect_readiness_sha256)):
            parser.error("package inspection cannot be mixed with legacy witness, report or readiness arguments")
    else:
        if any(pin is not None for pin in package_pins):
            parser.error("package pins require --inspect-source-package")
        if args.expect_report_sha256 is None or args.expect_readiness_sha256 is None:
            parser.error("--expect-report-sha256 and --expect-readiness-sha256 are required")
    if bool(args.inspect_witness) != bool(args.expect_witness_sha256):
        parser.error("witness input and independent witness SHA-256 are required together")
    if args.check_cutover and not args.inspect_witness:
        parser.error("cutover checks require an inspected witness and its independent SHA-256")
    if args.contract and args.inspect_witness:
        parser.error("choose contract generation or witness inspection")
    kwargs = {"expected_report_sha256": args.expect_report_sha256,
              "expected_readiness_sha256": args.expect_readiness_sha256}
    try:
        if args.inspect_source_package is not None:
            import price_source_package as package

            raw = package.regular(args.inspect_source_package, package.MAX_PACKAGE_BYTES)
            result = inspect_source_package(
                raw, args.expect_package_sha256, expected_source_commit=args.expect_source_commit,
                expected_source_tree=args.expect_source_tree, expected_rights_sha256=args.expect_rights_sha256)
        elif args.inspect_witness:
            raw = candidate.regular(args.inspect_witness)
            inspect = cutover_readiness if args.check_cutover else inspect_witness
            result = inspect(raw, args.expect_witness_sha256, args.source_root, **kwargs)
        else:
            result = build_witness(args.source_root, **kwargs)
            if args.contract:
                result = result["parity_cutover_prerequisites"]
        sys.stdout.buffer.write(cohort.encode(result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"price custody refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
