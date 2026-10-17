#!/usr/bin/env python3
"""Freeze and inspect the exact retained source/price cohort without granting admission."""

import argparse
import copy
import hashlib
from pathlib import Path
import re
import sys

import price_candidate as candidate
import price_manifest as manifest


ROOT = Path(__file__).absolute().parents[2]
ARTIFACTS = {
    "capture": ("research/commerce/governance/source-eight-capture-db92bf27-v1.json",
                "441fa612fdb5e9a2dd468a7da87730ff0ec53d6fcaa4b014337bcac393b1a909"),
    "proposal": ("research/commerce/governance/source-eight-mapping-db92bf27-v1.json",
                 "d499b6b7f5d01cfc4f1a9083faeca057f2a3d9511d070c3f7e101eee590e6f29"),
    "price_sidecar": ("research/commerce/governance/price-bounded-mapping-db92bf27-v1.json",
                      "300591b5db1f9b156ecf5282525e5d056c79986140cffff20fcd784b2e825792"),
    "supported_manifest": ("research/commerce/governance/price-supported-106-manifest-db92bf27-v1.tsv",
                           "451f0bb1a2f3acd3879ddc2945c14e74447ec6ac70dfd5ebf374d086c8f353dc"),
    "source_membership": ("research/commerce/governance/source-eight-membership-db92bf27-v1.json",
                          "d366bf0716df9455b6782c22c6157b5c38c543786bed4c4519a85796bfaf94cd"),
}
CORPUS_PATH = "adapter/web/tests/fixtures/price-contract.json"
FORMAT = "ecdev-source-price-cohort-v1"
SCOPE = "EXACT_RETAINED_SOURCE_AND_106_CASE_COHORT; NO_ENROLLMENT_OR_AUTHORITY"


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (candidate.canonical(value) + "\n").encode()


def require_pin(raw, pin):
    if (not isinstance(raw, bytes) or len(raw) > candidate.MAX_INPUT or type(pin) is not str
            or re.fullmatch("[0-9a-fA-F]{64}", pin) is None or digest(raw) != pin.lower()):
        raise ValueError("input does not match the independently supplied SHA-256")


def build_cohort(inputs, expected_pins, corpus_bytes):
    if set(inputs) != set(ARTIFACTS) or set(expected_pins) != set(ARTIFACTS):
        raise ValueError("all exact cohort artifacts and independent pins are required")
    for role, (_, frozen_pin) in ARTIFACTS.items():
        require_pin(inputs[role], expected_pins[role])
        if digest(inputs[role]) != frozen_pin:
            raise ValueError("input is outside the frozen retained cohort")
    capture, proposal, sidecar = (candidate.strict_json(inputs[role])
                                   for role in ("capture", "proposal", "price_sidecar"))
    selected = candidate.validate_mapping(proposal)
    binding = proposal["capture_binding"]
    if (binding["sha256"] != "sha256:" + digest(inputs["capture"])
            or binding["bytes"] != len(inputs["capture"])
            or binding["observed_head"] != capture["observed_head"]
            or proposal["source_semantics"]["source_graph"] != capture["source_graph"]
            or sidecar["source_mapping"]["sha256"] != digest(inputs["proposal"])
            or sidecar["source_mapping"]["capture_binding"] != binding
            or sidecar["source_bindings"] != selected):
        raise ValueError("capture, proposal and price sidecar are not one exact source cohort")
    membership = candidate.strict_json(inputs["source_membership"])
    members = {row["path"]: row for row in membership["inputs"]}
    if (membership["source_commit"] != capture["observed_head"]
            or membership["source_repository"] != capture["source_repository"]
            or len(members) != len(membership["inputs"]) or len(members) != 3053):
        raise ValueError("retained source membership identity or complete input set changed")
    for row in capture["files"]:
        content = bytes.fromhex(row["hex"])
        member = members[row["path"]]
        blob = hashlib.sha1(b"blob " + str(len(content)).encode() + b"\0" + content).hexdigest()
        if (row["sha256"] != digest(content) or member["sha256"] != row["sha256"]
                or member["bytes"] != len(content) or member["blob"] != blob):
            raise ValueError("captured payload differs from retained committed member")
    checked_manifest = manifest.inspect_manifest(corpus_bytes, inputs["price_sidecar"],
                                                expected_pins["price_sidecar"].lower(),
                                                inputs["supported_manifest"])
    coverage = sidecar["coverage"]
    indices = coverage["qualified_case_indices"]
    unavailable = coverage["unmapped_cases"]
    if (len(unavailable) != 2831 or len({row["index"] for row in unavailable}) != 2831
            or any(type(row["index"]) is not int or type(row["code"]) is not int
                   or row["target"] is not None for row in unavailable)
            or set(indices) & {row["index"] for row in unavailable}
            or set(indices) | {row["index"] for row in unavailable} != set(range(2937))):
        raise ValueError("the original supported and unmapped cases must form a complete partition")
    return {
        "format": FORMAT, "scope": SCOPE,
        "artifacts": {role: {"path": path, "sha256": digest(inputs[role]), "bytes": len(inputs[role])}
                      for role, (path, _) in ARTIFACTS.items()},
        "source_snapshot": {
            "source_id": capture["source_id"], "repository": capture["source_repository"],
            "observed_head": capture["observed_head"], "producer_schema_claim": capture["source_graph"]["schema"],
            "source_tree": membership["source_tree"],
            "membership_scope": membership["scope"],
            "history_completeness": membership["history_completeness"],
            "payloads": len(capture["files"]), "nodes": len(capture["source_graph"]["nodes"]),
            "edges": len(capture["source_graph"]["edges"]),
            "meaning": "Captured historical bytes; publication HEAD and current runtime are separate",
        },
        "source_bindings": copy.deepcopy(selected),
        "complete_proposal": {"target_custody": None, "target_assignments": 0,
                              "source_nodes": len(proposal["node_mappings"])},
        "bounded_price_reference": {
            "source_capability": sidecar["node_mappings"][0]["source_id"],
            "proposed_target": copy.deepcopy(sidecar["node_mappings"][0]["proposed_target"]),
            "target_contract": copy.deepcopy(sidecar["target_contract"]),
            "classification": "ONE_PROPOSED_106_CASE_TARGET_REFERENCE; NO_ACCEPTED_ASSIGNMENT",
        },
        "coverage": {
            "total": 2937, "supported": 106, "unmapped": 2831,
            "qualified_case_indices": copy.deepcopy(indices),
            "supported_manifest_sha256": digest(inputs["supported_manifest"]),
            "unmapped_cases_locator": "/coverage/unmapped_cases",
            "unmapped_cases_artifact": "price_sidecar",
            "unmapped_cases_canonical_sha256": digest(encode(unavailable)),
            "old99": copy.deepcopy(sidecar["old99"]),
            "new_candidate_indices": [row["index"] for row in sidecar["new_candidates"]],
            "manifest_inspection": checked_manifest,
        },
        "preservation": {
            "source_identity_codecs": ["ec1", "f1", "ECG"],
            "original_artifacts_rewritten": False,
            "original_case_fields": "Retained by exact corpus and sidecar byte pins; manifest selects index/name only",
            "source_namespace": "ecdev", "captured_source_not_current_source": True,
        },
        "status": {
            "source_authority": "SOURCE_ONLY", "canonical_enrollment": "BLOCKED",
            "source_lock_installed": False, "source_registered": False,
            "canonical_assignment": None, "accepted_owner": None, "authority": None,
            "authority_grants": [], "current_rights": None, "copying_authorization": "UNKNOWN",
            "world_admission": None, "admission": "PENDING", "source_adapter_cutover": False,
            "source_retirement": False, "native_runtime_acceptance": "NOT_ESTABLISHED",
        },
    }


def inspect_cohort(inputs, expected_pins, corpus_bytes, cohort_bytes, expected_cohort_sha256):
    require_pin(cohort_bytes, expected_cohort_sha256)
    expected = build_cohort(inputs, expected_pins, corpus_bytes)
    if candidate.canonical(candidate.strict_json(cohort_bytes)) != candidate.canonical(expected):
        raise ValueError("cohort identity, coverage, source history or pending authority changed")
    return {"format": "ecdev-source-price-cohort-inspection-v1", "status": "VALID_PENDING_COHORT",
            "cohort_sha256": digest(cohort_bytes), "supported": 106, "unmapped": 2831,
            "canonical_enrollment": "BLOCKED", "admission": "PENDING", "canonical_assignment": None}


def build_source_handoff(inputs, expected_pins, corpus_bytes):
    """Prepare source-owned identity evidence; absent central decisions remain explicit."""
    cohort = build_cohort(inputs, expected_pins, corpus_bytes)
    snapshot = cohort["source_snapshot"]
    target = cohort["bounded_price_reference"]["proposed_target"]
    return {
        "format": "ecdev-source-identity-authority-request-v1",
        "scope": "SOURCE_OWNED_REQUEST_ONLY; NOT_AN_ACCEPTED_CANONICAL_CONTRACT",
        "cohort": {"format": FORMAT, "sha256": digest(encode(cohort)),
                   "artifacts": copy.deepcopy(cohort["artifacts"])},
        "source_identity": {
            "source_id": snapshot["source_id"], "origin": snapshot["repository"], "namespace": "ecdev",
            "captured_commit": snapshot["observed_head"], "captured_tree": snapshot["source_tree"],
            "original_node_ids": {"artifact": "proposal", "locator": "/node_mappings",
                                  "count": 828, "codec": "ec1", "remap_performed": False},
            "original_records": {"artifact": "capture", "locator": "/files", "count": 734,
                                 "codecs": ["f1", "ECG"], "rewritten": False},
            "historical_aliases": {"artifact": "proposal", "locator": "/rights_claims",
                                   "count": 68, "rewritten": False},
            "commit_membership": {"artifact": "source_membership", "committed_inputs": 3053,
                                  "captured_payloads": 734, "uncaptured_inputs": 2319,
                                  "scope": snapshot["membership_scope"],
                                  "history_completeness": snapshot["history_completeness"]},
        },
        "registry_request": {"id": snapshot["source_id"], "origin": snapshot["repository"], "domain": None,
                             "status": "DOMAIN_AND_CANONICAL_BINDING_REQUIRE_CENTRAL_DECISION"},
        "authority_handoff": {
            "effective_source_authority": ".ecdev", "effective_state": "SOURCE_ONLY",
            "source_guard_sha256": "73d8b65f8bbe0a934ae83fec9628deb953ac8edb440e4b3c2fbc35801015f8e3",
            "requested_destination": "EXISTING_SINGLE_CANONICAL_WORLD_AUTHORITY_AND_HISTORY",
            "canonical_source_identity_mapping": None, "canonical_source_owner": None,
            "canonical_authority_acceptance": None, "accepted_shared_protocol_lock": None,
            "projection_installed": False, "source_lock_installed": False,
            "new_world_requested": False, "new_ui_requested": False, "handoff_effective": False,
        },
        "bounded_price_request": {
            "supported": 106, "unmapped": 2831, "total": 2937,
            "manifest_artifact": "supported_manifest", "sidecar_artifact": "price_sidecar",
            "original_source_bindings": copy.deepcopy(cohort["source_bindings"]),
            "existing_target_owner_reference": copy.deepcopy(target["existing_canonical_owner"]),
            "target_reference_meaning": "EXISTING_BOUNDED_CAPABILITY_REFERENCE; NOT_WHOLE_SOURCE_OWNER",
            "whole_source_owner": None, "accepted_owner": None, "canonical_assignment": None,
            "authority": None, "authority_grants": [], "current_rights": None,
            "copying_authorization": "UNKNOWN", "admission": "PENDING", "source_adapter_cutover": False,
        },
        "qualification_limits": {
            "canonical_source_graph_independently_rebuilt": False,
            "full_native_ui_asset_closure": "NOT_ESTABLISHED",
            "current_source_or_target_caller_admission": "NOT_ESTABLISHED",
            "source_retirement": False, "native_v1": "INCOMPLETE",
            "central_registry_decision": "NOT_SUPPLIED_BY_INSPECTED_PUBLISHED_CONTRACTS",
        },
    }


def inspect_source_handoff(inputs, expected_pins, corpus_bytes, request_bytes, expected_request_sha256):
    require_pin(request_bytes, expected_request_sha256)
    expected = build_source_handoff(inputs, expected_pins, corpus_bytes)
    if candidate.canonical(candidate.strict_json(request_bytes)) != candidate.canonical(expected):
        raise ValueError("source identity request or unresolved canonical authority changed")
    return {"format": "ecdev-source-identity-authority-request-inspection-v1",
            "status": "VALID_SOURCE_OWNED_REQUEST; CANONICAL_HANDOFF_NOT_ACCEPTED",
            "request_sha256": digest(request_bytes), "canonical_enrollment": "BLOCKED",
            "source_authority": "SOURCE_ONLY", "registry_domain": None, "handoff_effective": False}


def load_inputs(root):
    root = candidate.physical(root)
    return ({role: candidate.regular(root / path) for role, (path, _) in ARTIFACTS.items()},
            candidate.regular(root / CORPUS_PATH))


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    for role in ARTIFACTS:
        parser.add_argument("--expect-" + role.replace("_", "-") + "-sha256", required=True)
    parser.add_argument("--inspect-cohort", type=Path)
    parser.add_argument("--expect-cohort-sha256")
    parser.add_argument("--handoff-request", action="store_true")
    parser.add_argument("--inspect-request", type=Path)
    parser.add_argument("--expect-request-sha256")
    args = parser.parse_args()
    if bool(args.inspect_cohort) != bool(args.expect_cohort_sha256):
        parser.error("cohort input and independent cohort pin are required together")
    if bool(args.inspect_request) != bool(args.expect_request_sha256):
        parser.error("request input and independent request pin are required together")
    if sum(map(bool, (args.inspect_cohort, args.handoff_request, args.inspect_request))) > 1:
        parser.error("choose one cohort or handoff request operation")
    try:
        inputs, corpus = load_inputs(args.source_root)
        pins = {role: getattr(args, "expect_" + role + "_sha256") for role in ARTIFACTS}
        if args.inspect_cohort:
            value = inspect_cohort(inputs, pins, corpus, candidate.regular(args.inspect_cohort),
                                   args.expect_cohort_sha256)
        elif args.inspect_request:
            value = inspect_source_handoff(inputs, pins, corpus, candidate.regular(args.inspect_request),
                                           args.expect_request_sha256)
        elif args.handoff_request:
            value = build_source_handoff(inputs, pins, corpus)
        else:
            value = build_cohort(inputs, pins, corpus)
        sys.stdout.buffer.write(encode(value))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"cohort refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
