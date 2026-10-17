#!/usr/bin/env python3
"""Bind every frozen source identity to published responsibility review facets."""

import argparse
from collections import Counter
import copy
import hashlib
from pathlib import Path
import sys

import price_candidate as candidate
import price_cohort as cohort
import price_caller_readiness as caller


ROOT = Path(__file__).absolute().parents[2]
ANALYSIS_PATH = "research/commerce/governance/source-828-evidence-census-db92bf27-v1.json"
ANALYSIS_SHA256 = "1a55da96ee0b009818d3925f05c3bd89141a0094a14980f94b6ddd018718053b"
CONTRACTS_PATH = "research/commerce/governance/published-responsibility-contracts-d152d849-v1.json"
CONTRACTS_SHA256 = "2922b774fd0da98686b1d4be96ec4954651e3b5c8f621cccee33522ffda50dac"
SELECTED = {306: "donor_capability", 503: "source_caller", 513: "source_capability", 723: "source_technology"}


def equal(actual, expected, detail):
    if candidate.canonical(actual) != candidate.canonical(expected):
        raise ValueError(detail)


def load_pinned(root, path, pin):
    raw = candidate.regular(root / path)
    cohort.require_pin(raw, pin)
    return candidate.strict_json(raw)


def context(root):
    root = candidate.physical(root)
    inputs, raw_corpus = cohort.load_inputs(root)
    selected = cohort.build_cohort(inputs, {role: pin for role, (_, pin) in cohort.ARTIFACTS.items()}, raw_corpus)
    equal(load_pinned(root, caller.COHORT_PATH, caller.COHORT_SHA256), selected, "frozen cohort changed")
    capture = candidate.strict_json(inputs["capture"])
    proposal = candidate.strict_json(inputs["proposal"])
    membership = candidate.strict_json(inputs["source_membership"])
    analysis = load_pinned(root, ANALYSIS_PATH, ANALYSIS_SHA256)
    contracts = load_pinned(root, CONTRACTS_PATH, CONTRACTS_SHA256)
    validate_source(analysis, capture, proposal, membership)
    return selected, capture, proposal, analysis, contracts


def validate_source(analysis, capture, proposal, membership):
    """Independently bind identity, graph, payload and rights views to primary bytes.

    Declaration/ECG decoding and family analysis are pinned research inputs, not
    independently regenerated or re-attested by this reader.
    """
    nodes, edges = capture["source_graph"]["nodes"], capture["source_graph"]["edges"]
    equal(len(nodes), 828, "complete frozen source graph required")
    equal(len(edges), 1122, "complete source edge set required")
    equal([row["metadata"] for row in analysis["nodes"]], nodes, "original node metadata/order changed")
    equal([row["index"] for row in analysis["nodes"]], list(range(828)), "source index order changed")
    equal(analysis["edges"], [dict(edge, index=i) for i, edge in enumerate(edges)], "edge data/order changed")
    for i, (row, original) in enumerate(zip(analysis["nodes"], proposal["node_mappings"])):
        equal(row["metadata"]["id"], original["source_id"], "source identity changed")
        equal(row["capture_locator"], f"/source_graph/nodes/{i}", "source capture locator changed")
        equal(row["proposal_locator"], f"/node_mappings/{i}", "source mapping locator changed")
        equal(row["proposed_target"], None, "source analysis cannot admit a target")
        equal(original["proposed_target"], None, "frozen source proposal acquired an assignment")
        equal(row["incoming_edge_indices"], [j for j, e in enumerate(edges) if e["to"] == original["source_id"]],
              "incoming source lineage changed")
        equal(row["outgoing_edge_indices"], [j for j, e in enumerate(edges) if e["from"] == original["source_id"]],
              "outgoing source lineage changed")
    equal(proposal["target_custody"], None, "full source custody cannot be inferred")
    members = {row["path"]: (i, row) for i, row in enumerate(membership["inputs"])}
    equal(len(analysis["captured_payload_references"]), len(capture["files"]), "captured payload omitted")
    for i, (ref, file) in enumerate(zip(analysis["captured_payload_references"], capture["files"])):
        member_index, member = members[file["path"]]
        raw = bytes.fromhex(file["hex"])
        equal(ref, {"index": i, "path": file["path"], "sha256": cohort.digest(raw), "bytes": len(raw),
                    "git_blob": member["blob"], "mode": member["mode"], "capture_locator": f"/files/{i}",
                    "hex_locator": f"/files/{i}/hex", "membership_locator": f"/inputs/{member_index}"},
              "payload bytes, mode, identity or exact locator fields changed")
        equal(ref["sha256"], cohort.digest(raw), "payload bytes changed")
        equal(ref["sha256"], file["sha256"], "capture payload digest changed")
        equal(ref["sha256"], member["sha256"], "payload/member binding changed")
        equal(ref["bytes"], len(raw), "payload size changed")
        equal(ref["mode"], member["mode"], "payload mode changed")
        equal(ref["git_blob"], hashlib.sha1(b"blob " + str(len(raw)).encode() + b"\0" + raw).hexdigest(),
              "payload Git blob changed")
        equal(ref["git_blob"], member["blob"], "payload immutable membership changed")
    equal(analysis["history_and_rights"]["original_rights_claims"], proposal["rights_claims"],
          "original source rights claims changed")
    cohorts = analysis["evidence_cohorts"]
    all_indices = [i for row in cohorts for i in row["node_indices"]]
    equal(sorted(all_indices), list(range(828)), "evidence cohorts must be disjoint and complete")
    for row in cohorts:
        equal(row["count"], len(row["node_indices"]), "source cohort count changed")


def build(root=ROOT):
    selected, capture, proposal, analysis, contracts = context(root)
    structural = {node["id"]: node for node in capture["source_graph"]["nodes"] if node["path"]}
    routes = {row["source_semantic_key"]: row for row in contracts["source_family_facets"]}
    equal(sorted(routes), sorted(n["semantic_key"] for n in structural.values()), "all13 source routes required")
    for node in structural.values():
        equal(routes[node["semantic_key"]]["source_path"], node["path"], "source family path changed")
    facets = contracts["facet_catalog"]
    rows = []
    for node, original in zip(analysis["nodes"], proposal["node_mappings"]):
        family_ids = node["source_component_family_ids"]
        if any(identity not in structural for identity in family_ids):
            raise ValueError("source family must be an original structural identity")
        family_routes = [copy.deepcopy(routes[structural[identity]["semantic_key"]]) for identity in family_ids]
        # A family is mixed review context, never the implementation owner of
        # each member. Metadata responsibilities are separately explicit.
        metadata = ["assay-and-evidence", "repository-census-provenance"] if node["metadata"]["kind"] == "PROOF" \
            else ["repository-census-provenance"]
        codes = sorted(set(metadata + [code for route in family_routes for code in route["facet_codes"]]))
        if any(code not in facets for code in codes):
            raise ValueError("unpublished responsibility facet")
        bounded = None
        if node["index"] in SELECTED:
            role = SELECTED[node["index"]]
            equal(original["source_id"], selected["source_bindings"][role]["source_id"], "selected price identity changed")
            bounded = {"source_binding_role": role, "status": "PROPOSED_BOUNDED_CORRESPONDENCE; NOT_ADMITTED",
                       "source_slice": "price public API / products price-text call only; not the whole source node",
                       "bounded_API_qualified": 106, "bounded_API_unmapped": 2831,
                       "source_Web_qualified": 99, "source_Web_excluded_API_cases": 7,
                       "published_target_reference": copy.deepcopy(contracts["source_eight_review"]["candidate_price"]),
                       "admitted_owner": None, "target_exclusive_custody": None}
        rows.append({"index": node["index"], "original_pending_mapping": copy.deepcopy(original),
                     "source_evidence": copy.deepcopy(node), "metadata_responsibility_facets": metadata,
                     "source_family_review_contexts": family_routes, "candidate_facet_codes": codes,
                     "bounded_correspondence_proposal": bounded, "physical_implementation_owner": None,
                     "canonical_assignment": None, "admitted_owner": None, "target_custody": None,
                     "review_state": "PROPOSED_RESPONSIBILITY_REVIEW; NOT_ADMITTED"})
    return {"format": "ecdev-source-828-responsibility-candidates-v1",
            "scope": "COMPLETE_SOURCE_CENSUS_WITH_PROPOSED_REVIEW_FACETS; NO_OWNER_ASSIGNMENTS_OR_AUTHORITY",
            "inputs": {"frozen_cohort_sha256": caller.COHORT_SHA256, "source_analysis_sha256": ANALYSIS_SHA256,
                       "published_contracts_sha256": CONTRACTS_SHA256},
            "target_publication": copy.deepcopy(contracts["target"]),
            "source_snapshot": copy.deepcopy(selected["source_snapshot"]),
            "counts": {"nodes": len(rows), "edges": 1122, "captured_payloads": 734,
                       "source_kinds": dict(sorted(Counter(row["source_evidence"]["metadata"]["kind"] for row in rows).items())),
                       "source_roles": dict(sorted(Counter(row["source_evidence"]["source_role"] for row in rows).items())),
                       "source_families": 13, "contract_concepts": 26, "review_facets": len(facets),
                       "evidence_cohorts": len(analysis["evidence_cohorts"]), "bounded_price_references": 4,
                       "physical_owner_assignments": 0, "admitted_mappings": 0},
            "facet_catalog": copy.deepcopy(facets), "nodes": rows,
            "original_edges": copy.deepcopy(analysis["edges"]),
            "captured_payload_references": copy.deepcopy(analysis["captured_payload_references"]),
            "evidence_cohorts": copy.deepcopy(analysis["evidence_cohorts"]),
            "history_and_rights": copy.deepcopy(analysis["history_and_rights"]),
            "authority": copy.deepcopy(analysis["authority"]),
            "limits": ["Family facets are review context, not per-node physical ownership or runtime identity.",
                       "Only four original price bindings carry a published bounded reference; all828 admitted targets remain null.",
                       "Seven proof location-only families do not establish implementation custody; shared families remain shared.",
                       "Pinned source research/ECG analysis was not independently regenerated or re-attested by this reader.",
                       "Existing canonical ownership intent is established; concrete target mappings, custody, registry/domain/lock and Host evidence remain missing."]}


def inspect(raw, expected_sha256, root=ROOT):
    cohort.require_pin(raw, expected_sha256)
    equal(candidate.strict_json(raw), build(root), "source identity, lineage, responsibility or authority changed")
    return {"format": "ecdev-source-responsibility-inspection-v1", "status": "VALID_SOURCE_CANDIDATES_ONLY",
            "nodes": 828, "admitted_mappings": 0, "target_custody": None, "source_authority": "SOURCE_ONLY"}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--inspect", type=Path)
    parser.add_argument("--expect-sha256")
    args = parser.parse_args()
    if bool(args.inspect) != bool(args.expect_sha256):
        parser.error("inspection bytes and independent pin required together")
    try:
        result = inspect(candidate.regular(args.inspect), args.expect_sha256, args.source_root) if args.inspect \
            else build(args.source_root)
        sys.stdout.buffer.write(cohort.encode(result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"responsibility candidates refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
