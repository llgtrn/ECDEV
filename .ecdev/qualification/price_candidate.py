#!/usr/bin/env python3
"""Build caller-bound price candidates and bounded source-target mapping proposals.

Published owner declarations are custody data. Canonical assignment and admission
remain pending; target kernel execution and paid operations are outside this producer.
"""

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys


EUR_SHA256 = "d624d5596c8da53fd7be74335b923b4d728c83ec61034d1b79edba2da817d198"
OWNER_PATH = "research/commerce/governance/price-owner-membership-v1.json"
OWNER_SHA256 = "f7067406f77884b368160ef2b30d918d16da0424a7acc087f100d9ecef1c1f2f"
OWNER_BLOB = "510f1885637c9730624844c16ffb0c40dabbeaa1"
OWNER_SOURCE_SHA256 = "906c9c47b6fade06c2c570f10239551eb0dbe589eeca2cd73b2280876e815ade"
TARGET_COMMIT = "1a9c859ca5f1c27b69e99bddbbe47e1cf5496945"
TARGET_TREE = "6d8a8d3fe9d71ce97075ebf188b4e0a208a2724f"
CAPABILITY = "commerce.price.parse-text"
SOURCE_KEYS = {
    "donor_capability": ("donor-capability/ecdev/scrapinghub--price-parser/extract.price-public-api", "ec1:5f41ea7edd845d3559a926b897253666"),
    "source_capability": ("capability/extract.price-public-api", "ec1:a2e3b3e884070b64d4edacda08adf6e6"),
    "source_technology": ("technology/commerce.price-text", "ec1:e4aedb10fcd783728d5c4ab221f20132"),
    "source_caller": ("commerce.web-research", "ec1:9f15a2f5552ad0fc567975074fc9d86f"),
}
REASONS = ["TARGET_CUSTODY_PIN_NOT_PROVIDED", "CANONICAL_OWNER_DECISION_NOT_BOUND"]
MAX_INPUT = 16 * 1024 * 1024
TARGET_CONTRACT_PATH = "research/commerce/governance/price-target-owner-contract-v1.json"
TARGET_CONTRACT_SHA256 = "42c4eed4729032882cde433280431cec8887f829f3f522f109bc93b6079e4285"
TARGET_CONTRACT_BLOB = "89e449e57d4ad1ba5ee45c9d3c0f98e7af012455"
TARGET_CONTRACT_PUBLICATION = "4b5e163042c621702d7700b53087351f40fe192f"
TARGET_CONTRACT_PUBLICATION_TREE = "eb8977bc40ac8384f53449ece1b75af4b993ada0"
TARGET_REFERENCE_COMMIT = "d7c54f7ec836da6a8459f8a3786964ea7c674866"
TARGET_REFERENCE_TREE = "2965cd433ee1f744775b8cc885dc7cbab3efb6fb"
BOUNDED_FORMAT = "ecdev-bounded-price-mapping-v1"
BOUNDED_SCOPE = "PROPOSED_106_CASE_SUBDOMAIN_ONLY; NO_CANONICAL_ASSIGNMENT_OR_ADMISSION"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False)


def strict_json(raw):
    if not isinstance(raw, bytes) or len(raw) > MAX_INPUT:
        raise ValueError("bounded UTF-8 JSON bytes required")

    def unique(pairs):
        out = {}
        for key, value in pairs:
            if key in out:
                raise ValueError("duplicate JSON key: " + key)
            out[key] = value
        return out

    def invalid(value):
        raise ValueError("nonfinite JSON value: " + value)

    return json.loads(raw, object_pairs_hook=unique, parse_constant=invalid)


def physical(path):
    path = Path(path).absolute()
    if ".." in path.parts or any(p.is_symlink() for p in (*path.parents, path)):
        raise ValueError("input/output path contains a link or parent traversal")
    return path


def regular(path):
    path = physical(path)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_INPUT:
            raise ValueError("input must be a bounded regular file")
        with os.fdopen(descriptor, "rb", closefd=False) as stream:
            data = stream.read(MAX_INPUT + 1)
    finally:
        os.close(descriptor)
    if len(data) > MAX_INPUT:
        raise ValueError("input exceeds byte budget")
    return data


def reconcile(source_root):
    path = physical(Path(__file__).with_name("price_eur.py"))
    before = regular(path)
    if digest(before) != EUR_SHA256:
        raise ValueError("retained EUR qualification helper changed")
    helper = {"__file__": str(path), "__name__": "sealed_price_eur_candidate_input"}
    exec(compile(before, str(path), "exec"), helper)
    result = helper["reconcile"](physical(source_root))
    if regular(path) != before:
        raise ValueError("EUR helper changed during comparison")
    return result


def source_id(namespace, key):
    if type(namespace) is not str or type(key) is not str:
        raise ValueError("source identity namespace and key must be strings")
    h = hashlib.sha256()
    for part in (b"ecdev.node-id.v1", namespace.encode(), key.encode()):
        h.update(struct.pack(">Q", len(part)))
        h.update(part)
    return "ec1:" + h.digest()[:16].hex()


def validate_mapping(mapping):
    fields = {"format", "scope", "capture_binding", "target_custody", "unresolved_reasons", "node_mappings",
              "payloads", "rights_claims", "source_semantics", "global_identity_candidates", "candidate_scope"}
    if (not isinstance(mapping, dict) or set(mapping) != fields
            or mapping["format"] != "ecdev-source-mapping-proposal-v1"
            or mapping["scope"] != "PROPOSAL_ONLY; NOT_ENROLLED"
            or mapping["target_custody"] is not None or mapping["unresolved_reasons"] != REASONS):
        raise ValueError("source mapping must remain a complete pending proposal")
    binding = mapping["capture_binding"]
    semantics = mapping["source_semantics"]
    if (binding["format"] != "ecdev-source-capture-v1" or binding["source_id"] != "ecdev"
            or binding["source_repository"] != "llgtrn/ECDEV"
            or re.fullmatch(r"sha256:[0-9a-f]{64}", binding["sha256"]) is None
            or semantics["capture_sha256"] != binding["sha256"][7:]
            or semantics["observed_head"] != binding["observed_head"]
            or semantics["canonical_enrollment"] != "BLOCKED"
            or semantics["canonical_identity_mapping"] is not None
            or semantics["norl_relevance_mapping"] is not None):
        raise ValueError("source capture binding or pending semantics changed")
    nodes = semantics["source_graph"]["nodes"]
    edges = semantics["source_graph"]["edges"]
    mappings = mapping["node_mappings"]
    if (type(mappings) is not list or len(mappings) != 828 or len(nodes) != 828 or len(edges) != 1122
            or semantics["active_evidence_records"] != 726 or semantics["retired_evidence_records"] != 8
            or len(semantics["facts"]) != 1861):
        raise ValueError("current source coverage differs from 734/828/1122 capture")
    by_key, ids = {}, set()
    for index, (mapped, node) in enumerate(zip(mappings, nodes)):
        key = (mapped["source_namespace"], mapped["source_semantic_key"])
        identity = mapped["source_id"]
        if (key in by_key or identity in ids or identity != source_id(*key)
                or mapped["source_metadata"] != node
                or (node["namespace"], node["semantic_key"], node["id"]) != (*key, identity)
                or mapped["capture_locator"] != f"/source_graph/nodes/{index}"
                or mapped["proposed_target"] is not None or mapped["unresolved_reasons"] != REASONS):
            raise ValueError("source identity, metadata, coverage or null assignment changed")
        by_key[key] = (index, mapped)
        ids.add(identity)
    for edge in edges:
        if edge["from"] not in ids or edge["to"] not in ids:
            raise ValueError("source graph edge has an absent endpoint")
    records = semantics["records"]
    if len(records) != 734 or len({row["path"] for row in records}) != 734:
        raise ValueError("all source payload records required")
    if [row["path"] for row in records] != sorted(row["path"] for row in records):
        raise ValueError("source payload records reordered")
    lanes = mapping["payloads"]
    if set(lanes) != {"declaration", "history", "evidence", "knowledge"}:
        raise ValueError("all four source payload lanes required")
    payloads = [row for rows in lanes.values() for row in rows]
    if len(payloads) != 734 or len({row["source_path"] for row in payloads}) != 734:
        raise ValueError("source payload mapping coverage changed")
    positions = {row["path"]: (i, row) for i, row in enumerate(records)}
    for lane, rows in lanes.items():
        directory = "declared" if lane == "declaration" else lane
        for row in rows:
            path = row["source_path"]
            if path not in positions or not path.startswith(f".ecdev/{directory}/"):
                raise ValueError("source payload lane or path changed")
            index, record = positions[path]
            if (row["capture_locator"] != f"/files/{index}" or row["hex_locator"] != f"/files/{index}/hex"
                    or row["sha256"] != record["sha256"] or row["bytes"] != record["bytes"]):
                raise ValueError("source payload digest or original locator changed")
    rights, donors = mapping["rights_claims"], semantics["donors"]
    if len(rights) != 68 or len(donors) != 68 or len({r["source_alias"] for r in rights}) != 68:
        raise ValueError("all source aliases and rights claims required")
    for claim, donor in zip(rights, donors):
        if (any(claim[k] != donor[v] for k, v in (("source_alias", "source_alias"), ("source_id", "source_id"),
                ("origin", "origin"), ("license_claim", "license"), ("provenance", "provenance")))
                or claim["scope"] != "RECORDED_SOURCE_CLAIMS; NO_CURRENT_RIGHTS_AUTHORIZATION"
                or donor["canonical_effective_state"] is not None):
            raise ValueError("source rights or effective-state claim changed")
    selected = {}
    for role, (key, identity) in SOURCE_KEYS.items():
        index, mapped = by_key[("ecdev", key)]
        if mapped["source_id"] != identity:
            raise ValueError("price source identity changed")
        selected[role] = {"source_id": identity, "source_namespace": "ecdev", "source_semantic_key": key,
                          "mapping_locator": f"/node_mappings/{index}", "capture_locator": mapped["capture_locator"]}
    caller = by_key[("ecdev", SOURCE_KEYS["source_caller"][0])][1]["source_metadata"]
    if caller["path"] != "adapter/web" or "extract.price-public-api" not in caller["capabilities"]:
        raise ValueError("real source price caller is absent")
    return selected


def git_object(kind, body):
    return hashlib.sha1(kind.encode() + b" " + str(len(body)).encode() + b"\0" + body).hexdigest()


def validate_owner(witness, target):
    if (witness["format"] != "ecdev-price-owner-membership-v1"
            or witness["commit"] != TARGET_COMMIT or witness["tree"] != TARGET_TREE
            or witness["scope"] != "DECLARED_SOURCE_NODE_MEMBERSHIP_ONLY; NO_ADMISSION_RIGHTS_OR_BUILD_ATTESTATION"):
        raise ValueError("owner proof pin or scope changed")
    commit = witness["commit_preimage_utf8"].encode()
    if git_object("commit", commit) != TARGET_COMMIT or not commit.startswith(f"tree {TARGET_TREE}\n".encode()):
        raise ValueError("owner commit preimage mismatch")
    rows = witness["tree_witness"]
    trees = {row["path"]: row for row in rows}
    if len(rows) != 3 or len(trees) != 3 or trees[""]["oid"] != TARGET_TREE:
        raise ValueError("all three unique owner ancestor trees required")
    for row in rows:
        entries = row["entries"]
        if len({e["name"] for e in entries}) != len(entries):
            raise ValueError("duplicate owner tree member")
        for e in entries:
            if (not e["name"] or e["name"] in (".", "..") or "/" in e["name"] or "\\" in e["name"]
                    or (e["mode"], e["type"]) not in (("040000", "tree"), ("100644", "blob"), ("100755", "blob"))
                    or re.fullmatch("[0-9a-f]{40}", e["oid"]) is None):
                raise ValueError("invalid owner tree member")
        ordered = sorted(entries, key=lambda e: e["name"].encode() + (b"/" if e["type"] == "tree" else b""))
        body = b"".join(e["mode"].lstrip("0").encode() + b" " + e["name"].encode() + b"\0"
                        + bytes.fromhex(e["oid"]) for e in ordered)
        if git_object("tree", body) != row["oid"]:
            raise ValueError("owner ancestor tree hash mismatch")
    file = witness["file"]
    body = file["content_utf8"].encode()
    if (file["encoding"] != "UTF8_TEXT_FOR_MEMBERSHIP_PROOF_ONLY" or file["mode"] != "100644"
            or file["bytes"] != len(body) or file["bytes"] != 75376
            or file["sha256"] != OWNER_SOURCE_SHA256 or digest(body) != OWNER_SOURCE_SHA256
            or file["git_blob"] != OWNER_BLOB or git_object("blob", body) != OWNER_BLOB):
        raise ValueError("pinned owner declaration bytes or blob changed")
    directory = ""
    parts = file["path"].split("/")
    if len(parts) != 3 or set(trees) != {"", parts[0], "/".join(parts[:2])}:
        raise ValueError("owner file ancestor coverage mismatch")
    for index, name in enumerate(parts):
        entry = next(e for e in trees[directory]["entries"] if e["name"] == name)
        if index == len(parts) - 1:
            if (entry["oid"], entry["mode"], entry["type"]) != (OWNER_BLOB, "100644", "blob"):
                raise ValueError("owner declaration is not a member of the pinned tree")
        else:
            directory = "/".join(parts[:index + 1])
            if entry["type"] != "tree" or entry["oid"] != trees[directory]["oid"]:
                raise ValueError("owner ancestor membership mismatch")
    owner = target["owner"]
    marker = '            key: "' + owner["key"] + '",'
    text = file["content_utf8"]
    if text.count(marker) != 1:
        raise ValueError("declared owner key is absent or ambiguous")
    start = text.rfind("        Node {", 0, text.index(marker))
    end = text.index("\n        },", text.index(marker))
    block = text[start:end]
    fields = {}
    for name in ("key", "path", "canonical_path"):
        match = re.search(r'^\s*' + name + r': "([^"\n]+)",$', block, re.MULTILINE)
        if match is None:
            raise ValueError("missing declared owner field")
        fields[name] = match.group(1)
    provides = re.search(r'provides:\s*&\[([^]]*)\]', block)
    if (fields != {"key": owner["key"], "path": owner["path"], "canonical_path": owner["path"]}
            or "kind: NodeKind::Physiology," not in block or "lifecycle: NodeLifecycle::Active," not in block
            or provides is None or CAPABILITY not in re.findall(r'"([^"\n]+)"', provides.group(1))
            or target["capability"] != CAPABILITY):
        raise ValueError("declared owner path or provided capability changed")
    return {"status": "VERIFIED_DECLARED_SOURCE_NODE_MEMBERSHIP_ONLY", "owner": copy.deepcopy(owner),
            "repository": witness["repository"], "commit": TARGET_COMMIT, "tree": TARGET_TREE,
            "declaration_path": file["path"], "declaration_git_blob": OWNER_BLOB,
            "declaration_sha256": OWNER_SOURCE_SHA256, "provided_capability": CAPABILITY,
            "scope": witness["scope"], "current_owner_acceptance": None}


def build_candidate(mapping_bytes, expected_mapping_sha256, owner_witness_bytes, source_root=None):
    if (not isinstance(mapping_bytes, bytes) or not isinstance(owner_witness_bytes, bytes)
            or len(mapping_bytes) > MAX_INPUT or len(owner_witness_bytes) > MAX_INPUT
            or type(expected_mapping_sha256) is not str or re.fullmatch("[0-9a-fA-F]{64}", expected_mapping_sha256) is None
            or digest(mapping_bytes) != expected_mapping_sha256.lower()):
        raise ValueError("mapping does not match the independently supplied SHA-256")
    if digest(owner_witness_bytes) != OWNER_SHA256:
        raise ValueError("retained owner membership proof changed")
    mapping, witness = strict_json(mapping_bytes), strict_json(owner_witness_bytes)
    selected = validate_mapping(mapping)
    root = Path(__file__).absolute().parents[2] if source_root is None else source_root
    comparison = reconcile(root)
    if (comparison["total"], comparison["supported"], comparison["unmapped"], comparison["new_candidate_count"]) != (2937, 106, 2831, 7):
        raise ValueError("complete sealed price coverage required")
    target = copy.deepcopy(comparison["new_candidates"][0]["target"])
    owner = validate_owner(witness, target)
    if (comparison["canonical_assignment"] is not None or comparison["accepted_owner"] is not None
            or comparison["authority_grants"] or comparison["admission"] != "PENDING"
            or comparison["copying_authorization"] != "UNKNOWN"
            or comparison["source_adapter_cutover"] or comparison["governed_caller_locally_run"]):
        raise ValueError("retained price comparison cannot self-grant admission")
    return {"format": "ecdev-price-capability-candidate-v1", "scope": "BOUNDED_PENDING_CAPABILITY_CANDIDATE; NO_TARGET_ASSIGNMENT_OR_ADMISSION",
            "source_mapping": {"sha256": digest(mapping_bytes), "format": mapping["format"],
                               "capture_binding": copy.deepcopy(mapping["capture_binding"]), "source_nodes": 828,
                               "target_assignments": 0, "target_custody": None,
                               "validation": "CALLER_BOUND_PRODUCER_OUTPUT; NO_INDEPENDENT_SOURCE_ASSESSMENT"},
            "source_bindings": selected,
            "source_caller": {"entry": "adapter/web/src/lib.rs::products", "call": "price::parse_price(Some(raw), stated_priceCurrency, None, None)",
                              "output": "products[].observed_offers[].price_text_interpretation", "used_for_observed_price": False,
                              "raw_price_preserved": True, "currency_token_is_iso_identity": False},
            "target_binding": target, "declared_owner_verification": owner,
            "owner_membership_artifact": {"path": OWNER_PATH, "sha256": OWNER_SHA256},
            "target_publication": copy.deepcopy(comparison["target_publication"]),
            "coverage": {key: copy.deepcopy(comparison[key]) for key in ("total", "supported", "unmapped", "new_candidate_count",
                         "qualified_case_indices", "unmapped_cases", "new_manifest_sha256", "all106_manifest_sha256", "reason_only_changes")},
            "old99": copy.deepcopy(comparison["old99"]), "new_candidates": copy.deepcopy(comparison["new_candidates"]),
            "canonical_assignment": None, "accepted_owner": None, "authority_grants": [],
            "copying_authorization": "UNKNOWN", "current_rights": None, "build_assessment": None, "binary_attestation": None,
            "world_admission": None, "admission": "PENDING", "source_adapter_cutover": False, "governed_caller_locally_run": False,
            "source8_registry": {"status": "PENDING", "lock_installed": False, "granted_by_this_candidate": False}}


def inspect_candidate(mapping_bytes, expected_mapping_sha256, owner_witness_bytes, candidate_bytes, source_root=None):
    expected = build_candidate(mapping_bytes, expected_mapping_sha256, owner_witness_bytes, source_root)
    actual = strict_json(candidate_bytes)
    if canonical(actual) != canonical(expected):
        raise ValueError("price candidate binding, coverage, rights or pending state changed")
    return {"format": "ecdev-price-capability-candidate-inspection-v1", "status": "VALID_PENDING_CAPABILITY_CANDIDATE",
            "candidate_sha256": digest(candidate_bytes), "source_mapping_sha256": digest(mapping_bytes),
            "supported": 106, "unmapped": 2831, "total": 2937, "admission": "PENDING", "canonical_assignment": None}


def validate_target_contract(raw, expected_sha256, prior):
    if (not isinstance(raw, bytes) or len(raw) > MAX_INPUT or type(expected_sha256) is not str
            or re.fullmatch("[0-9a-fA-F]{64}", expected_sha256) is None
            or digest(raw) != expected_sha256.lower()):
        raise ValueError("target contract does not match the independently supplied SHA-256")
    if digest(raw) != TARGET_CONTRACT_SHA256 or git_object("blob", raw) != TARGET_CONTRACT_BLOB:
        raise ValueError("target contract is not the reviewed immutable version")
    contract = strict_json(raw)
    target, pin = contract["target"], contract["target_repository_pin"]
    previous = prior["target_binding"]
    if (contract["classification"] != "TARGET_OWNED_EXISTING_OWNER_AND_BOUNDED_MAPPING_REFERENCE; NOT_ENGINE_ENROLLMENT_SCHEMA_OR_RUNTIME_ADMISSION"
            or (pin["commit"], pin["tree"]) != (TARGET_REFERENCE_COMMIT, TARGET_REFERENCE_TREE)
            or (target["repository"], target["commit"], target["tree"]) != (pin["repository"], pin["commit"], pin["tree"])):
        raise ValueError("reviewed target source reference changed")
    for target_key, prior_key in (("capability", "capability"), ("canonical_capability_key", "canonical_capability_key"),
                                  ("capability_NodeId", "capability_NodeId"), ("implementation_key", "implementation_key"),
                                  ("implementation_NodeId", "implementation_NodeId"), ("source_artifact", "artifact"),
                                  ("existing_canonical_owner", "owner"), ("maintained_caller", "caller")):
        if target[target_key] != previous[prior_key]:
            raise ValueError("reviewed target identity or existing owner differs from qualified subset")
    source = contract["source"]
    for key, role in (("donor_capability", "donor_capability"), ("native_source_capability", "source_capability"),
                      ("technology", "source_technology")):
        if source[key] != prior["source_bindings"][role]["source_id"]:
            raise ValueError("target contract names another source capability")
    coverage = contract["bounded_coverage"]
    if (tuple(coverage[key] for key in ("total", "supported", "unmapped")) != (2937, 106, 2831)
            or coverage["new_indices"] != [c["index"] for c in prior["new_candidates"]]
            or coverage["case_map"]["sha256"] != "b8caf5500745de277cfb9d011722b05e884a0e8e76671e0e849756720f8cd16d"
            or coverage["source_corpus_sha256"] != source["corpus_sha256"]):
        raise ValueError("reviewed target contract exceeds the qualified price subdomain")
    owner = prior["declared_owner_verification"]
    declared = contract["pinned_target_inputs"][owner["declaration_path"]]
    if (declared["sha256"], declared["git_blob"]) != (OWNER_SOURCE_SHA256, OWNER_BLOB):
        raise ValueError("current target declaration differs from the immutable owner witness")
    status = contract["status_dimensions"]
    if (status["source_canonical_assignment"] is not None or status["source_accepted_owner"] is not None
            or status["source_adapter_cutover"] or status["source_copying_authorization"] != "UNKNOWN"
            or status["current_external_runtime_admission"] != "NOT_ESTABLISHED"
            or status["exclusive_execution_custody"] != "NOT_ESTABLISHED"
            or contract["no_grant"]["authority"] or contract["no_grant"]["rights_assessed"]
            or contract["no_grant"]["source_enrollment_completed"]):
        raise ValueError("target source contract cannot grant authority, rights or enrollment")
    return contract


def build_bounded_mapping(mapping_bytes, expected_mapping_sha256, owner_witness_bytes,
                          target_contract_bytes, expected_target_contract_sha256, source_root=None):
    """Propose one concrete target only in this separate, explicitly bounded v1 format."""
    prior = build_candidate(mapping_bytes, expected_mapping_sha256, owner_witness_bytes, source_root)
    contract = validate_target_contract(target_contract_bytes, expected_target_contract_sha256, prior)
    target = copy.deepcopy(contract["target"])
    source = copy.deepcopy(prior["source_bindings"]["source_capability"])
    source["scope"] = "SUPPORTED_106_CASE_SUBDOMAIN_ONLY; ENTIRE_SOURCE_CAPABILITY_NOT_ADOPTED"
    source["proposed_target"] = target
    new_candidates = []
    for candidate in prior["new_candidates"]:
        row = {key: copy.deepcopy(value) for key, value in candidate.items() if key != "target"}
        row["proposed_target_locator"] = "/node_mappings/0/proposed_target"
        new_candidates.append(row)
    return {"format": BOUNDED_FORMAT, "scope": BOUNDED_SCOPE,
            "source_mapping": copy.deepcopy(prior["source_mapping"]),
            "source_bindings": copy.deepcopy(prior["source_bindings"]),
            "source_caller": copy.deepcopy(prior["source_caller"]),
            "node_mappings": [source],
            "target_contract": {"path": TARGET_CONTRACT_PATH, "sha256": digest(target_contract_bytes),
                                "git_blob": TARGET_CONTRACT_BLOB, "publication_commit": TARGET_CONTRACT_PUBLICATION,
                                "publication_tree": TARGET_CONTRACT_PUBLICATION_TREE},
            "target_custody": {"status": "PUBLISHED_TARGET_SOURCE_REFERENCE_ONLY",
                               "target_repository_pin": copy.deepcopy(contract["target_repository_pin"]),
                               "runtime_custody": "NOT_ESTABLISHED", "exclusive_execution_custody": "NOT_ESTABLISHED",
                               "current_external_runtime_admission": "NOT_ESTABLISHED",
                               "build_assessment": None, "binary_attestation": None, "current_rights": None},
            "target_owner_statement": contract["owner_statement"],
            "declared_owner_verification": copy.deepcopy(prior["declared_owner_verification"]),
            "owner_membership_artifact": copy.deepcopy(prior["owner_membership_artifact"]),
            "target_contract_status_dimensions": copy.deepcopy(contract["status_dimensions"]),
            "target_reported_caller_evidence": {"classification": "RECORDED_PUBLISHED_TARGET_REPORT; NOT_LOCALLY_RERUN_OR_INDEPENDENT_EXECUTION_ATTESTATION",
                                                "evidence": copy.deepcopy(contract["caller_evidence"])},
            "coverage": copy.deepcopy(prior["coverage"]), "old99": copy.deepcopy(prior["old99"]),
            "new_candidates": new_candidates,
            "historical_subset_publication": copy.deepcopy(prior["target_publication"]),
            "exact_input_output_and_admission": copy.deepcopy(contract["exact_input_output_and_admission"]),
            "canonical_assignment": None, "accepted_owner": None, "authority": None, "authority_grants": [],
            "copying_authorization": "UNKNOWN", "current_rights": None, "build_assessment": None,
            "binary_attestation": None, "world_admission": None, "admission": "PENDING",
            "source_adapter_cutover": False, "governed_caller_locally_run": False,
            "source8_registry": copy.deepcopy(prior["source8_registry"])}


def inspect_bounded_mapping(mapping_bytes, expected_mapping_sha256, owner_witness_bytes,
                            target_contract_bytes, expected_target_contract_sha256, bounded_mapping_bytes,
                            source_root=None):
    expected = build_bounded_mapping(mapping_bytes, expected_mapping_sha256, owner_witness_bytes,
                                     target_contract_bytes, expected_target_contract_sha256, source_root)
    if canonical(strict_json(bounded_mapping_bytes)) != canonical(expected):
        raise ValueError("bounded price mapping target, scope, coverage or pending authority changed")
    return {"format": "ecdev-bounded-price-mapping-inspection-v1", "status": "VALID_PENDING_BOUNDED_MAPPING",
            "mapping_sha256": digest(bounded_mapping_bytes), "source_mapping_sha256": digest(mapping_bytes),
            "target_contract_sha256": digest(target_contract_bytes), "proposed_targets": 1,
            "supported": 106, "unmapped": 2831, "total": 2937, "admission": "PENDING",
            "canonical_assignment": None, "accepted_owner": None, "authority": None}


def run(mapping_path, expected_mapping_sha256, output, source_root=None):
    root = physical(Path(__file__).absolute().parents[2] if source_root is None else source_root)
    mapping_path, owner_path, output = physical(mapping_path), physical(root / OWNER_PATH), physical(output)
    protected = (root, Path(__file__).absolute().parents[2], mapping_path, owner_path)
    if output.exists() or output.is_symlink() or any(output == p or output.is_relative_to(p) or p.is_relative_to(output) for p in protected):
        raise ValueError("output must be new and disjoint from all source and input paths")
    mapping, owner = regular(mapping_path), regular(owner_path)
    candidate = build_candidate(mapping, expected_mapping_sha256, owner, root)
    raw = (canonical(candidate) + "\n").encode()
    checked = inspect_candidate(mapping, expected_mapping_sha256, owner, raw, root)
    if regular(mapping_path) != mapping or regular(owner_path) != owner:
        raise ValueError("candidate input changed during validation")
    output.mkdir()
    (output / "price-capability-candidate-v1.json").write_bytes(raw)
    (output / "inspection.json").write_text(canonical(checked) + "\n")
    return checked


def run_bounded_mapping(mapping_path, expected_mapping_sha256, target_contract_path,
                        expected_target_contract_sha256, output, source_root=None):
    root = physical(Path(__file__).absolute().parents[2] if source_root is None else source_root)
    mapping_path, target_path = physical(mapping_path), physical(target_contract_path)
    owner_path, output = physical(root / OWNER_PATH), physical(output)
    protected = (root, Path(__file__).absolute().parents[2], mapping_path, target_path, owner_path)
    if output.exists() or output.is_symlink() or any(output == p or output.is_relative_to(p) or p.is_relative_to(output) for p in protected):
        raise ValueError("output must be new and disjoint from all source and input paths")
    mapping, owner, target = regular(mapping_path), regular(owner_path), regular(target_path)
    candidate = build_bounded_mapping(mapping, expected_mapping_sha256, owner, target,
                                      expected_target_contract_sha256, root)
    raw = (canonical(candidate) + "\n").encode()
    checked = inspect_bounded_mapping(mapping, expected_mapping_sha256, owner, target,
                                      expected_target_contract_sha256, raw, root)
    if regular(mapping_path) != mapping or regular(owner_path) != owner or regular(target_path) != target:
        raise ValueError("bounded mapping input changed during validation")
    output.mkdir()
    (output / "bounded-price-mapping-v1.json").write_bytes(raw)
    (output / "bounded-inspection.json").write_text(canonical(checked) + "\n")
    return checked


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--mapping", required=True, type=Path)
    parser.add_argument("--expect-mapping-sha256", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--target-contract", type=Path)
    parser.add_argument("--expect-target-contract-sha256")
    parser.add_argument("--source-root", type=Path, default=Path(__file__).absolute().parents[2])
    args = parser.parse_args()
    try:
        if (args.target_contract is None) != (args.expect_target_contract_sha256 is None):
            raise ValueError("target contract and independent target SHA-256 are required together")
        if args.target_contract is None:
            checked = run(args.mapping, args.expect_mapping_sha256, args.output, args.source_root)
        else:
            checked = run_bounded_mapping(args.mapping, args.expect_mapping_sha256, args.target_contract,
                                           args.expect_target_contract_sha256, args.output, args.source_root)
        print(canonical(checked))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError, StopIteration) as error:
        print(f"candidate refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
