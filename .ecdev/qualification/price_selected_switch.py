#!/usr/bin/env python3
"""Prepare and inspect a disabled switch contract for the selected source caller."""

import argparse
import copy
from pathlib import Path
import sys

import price_candidate as candidate
import price_cohort as cohort
import price_custody as custody
import price_target_evidence as target


ROOT = Path(__file__).absolute().parents[2]
RIGHTS_PATH = "research/commerce/governance/price-selected-source-rights-provenance-v1.json"
RIGHTS_SHA256 = "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410"
ENROLLMENT_PATH = "research/commerce/governance/price-current-enrollment-prerequisites-a54d9c09-v1.json"
ENROLLMENT_SHA256 = "b1148d34fcf4358b40e5b4ffc65019803bae8982e3469d1ff69faea5e459322b"
CALLER_PATHS = ("apps/server/src/lib.rs", "adapter/web/src/lib.rs", "adapter/web/src/document.rs",
                "domain/commerce/src/service.rs", "apps/server/src/web.rs")
OBLIGATIONS = ("target_execution", "exclusive_custody", "authority", "unknown_effect",
               "rights_and_enrollment", "production_cutover")


def same(actual, expected, message):
    if candidate.canonical(actual) != candidate.canonical(expected):
        raise ValueError(message)


def pinned(root, path, pin, independent=None):
    raw = candidate.regular(root / path)
    if independent is not None:
        cohort.require_pin(raw, independent)
    cohort.require_pin(raw, pin)
    return candidate.strict_json(raw)


def selector_groups(vectors):
    """Group exact source request tuples, preserving every duplicate case identity."""
    groups = {}
    for row in vectors:
        request = row["source_request"]
        if type(request["input"]) is not str or request["decimal_separator"] is not None \
                or request["digit_group_separator"] is not None:
            raise ValueError("selected caller only supplies Some string input and no explicit separators")
        hint = {"present": request["currency_hint"] is not None, "value": request["currency_hint"]}
        signature = {"input_utf8_hex": request["input"].encode().hex(), "currency_hint": hint,
                     "decimal_separator": None, "digit_group_separator": None}
        key = candidate.canonical(signature)
        fields = row["expected_source_four_fields"]
        if key not in groups:
            groups[key] = {"request_signature": signature, "source_case_indices": [], "source_case_names": [],
                           "expected_source_four_fields": copy.deepcopy(fields),
                           "expected_source_float_bits": row["expected_source_float_bits"],
                           "text_selector_facts": copy.deepcopy(row["text_selector_facts"]),
                           "input_is_none_fact_present": False}
        group = groups[key]
        same(group["expected_source_four_fields"], fields, "duplicate source request has different output")
        same(group["expected_source_float_bits"], row["expected_source_float_bits"], "duplicate float bits differ")
        group["source_case_indices"].append(row["index"])
        group["source_case_names"].append(row["name"])
    return list(groups.values())


def build_contract(root=ROOT, *, expected_target_sha256, expected_rights_sha256):
    root = candidate.physical(root)
    if type(expected_rights_sha256) is not str:
        raise ValueError("independent source rights SHA-256 required")
    assessment = target.build_assessment(root, expected_target_sha256=expected_target_sha256)
    same(assessment["status"], "TEST_EVIDENCE_RECONCILED_ONLY", "only bounded test evidence is consumable")
    rights = pinned(root, RIGHTS_PATH, RIGHTS_SHA256, expected_rights_sha256)
    for material in rights["current_source_material"]:
        raw = candidate.regular(root / material["path"])
        same({"sha256": cohort.digest(raw), "bytes": len(raw)},
             {"sha256": material["sha256"], "bytes": material["bytes"]}, "source rights material changed")
    enrollment = pinned(root, ENROLLMENT_PATH, ENROLLMENT_SHA256)
    witness = custody.build_witness(root, expected_report_sha256=custody.REPORT_SHA256,
                                    expected_readiness_sha256=custody.READINESS_SHA256)
    prerequisite = witness["parity_cutover_prerequisites"]
    groups = selector_groups(prerequisite["vectors"])
    same(len(groups), 96, "all99 identities must retain their exact96 distinct request tuples")
    same(sorted(i for row in groups for i in row["source_case_indices"]),
         prerequisite["selected_web_indices"], "caller tuple partition changed")
    obligations = []
    for row in prerequisite["obligations"]:
        key = row["key"]
        obligations.append({"key": key, "original_evidence": None, "production_acceptance": "OPEN",
                            "original_requirements": copy.deepcopy(row["requires"]),
                            "reconciled_test_scope": copy.deepcopy(assessment["obligation_assessment"][key]),
                            "source_owned_rights_and_provenance": copy.deepcopy(rights["source_owned_closures"])
                            if key == "rights_and_enrollment" else None})
    same([row["key"] for row in obligations], list(OBLIGATIONS), "exact six acceptance obligations required")
    entries = custody.tree_entries(root, custody.CURRENT_COMMIT, custody.CURRENT_TREE)
    caller_files = [custody.committed_file(root, entries, path, include_hex=False, worktree=True)
                    for path in CALLER_PATHS]
    return {"format": "ecdev-selected-price-caller-switch-contract-v1",
            "classification": "PREPARED_SOURCE_CALLER_SWITCH; DISABLED_PENDING_GENUINE_PRODUCTION_ACCEPTANCE",
            "inputs": {"target_report_sha256": target.TARGET_SHA256, "source_rights_sha256": RIGHTS_SHA256,
                       "source_witness_sha256": "e6f58d934cc1b55d949dc925f83510f5361432f64a3b5620424da3a01bc8d2b0",
                       "enrollment_readback_sha256": ENROLLMENT_SHA256},
            "selected_source_bindings": copy.deepcopy(assessment["selected_source"]["bindings"]),
            "real_source_caller_files": caller_files,
            "selected_boundary": "products: offer.price.as_str -> parse_price(Some(raw),priceCurrency.as_str,None,None)",
            "current_provider_chain": ["configured_engine installs native-web Provider",
                                       "Web.acquire/reextract calls document.extract",
                                       "document.extract calls extract_with_hash",
                                       "extract_with_hash calls products",
                                       "products interprets string offer prices independently of observed Money"],
            "selector_groups": groups, "selected_case_indices": copy.deepcopy(prerequisite["selected_web_indices"]),
            "counts": {"selected_Web_cases": 99, "exact_distinct_request_tuples": 96,
                       "API_only_excluded": 7, "full_API_supported": 106, "full_API_unmapped": 2831,
                       "full_API_total": 2937, "Web_shaped_unmapped": 2611, "non_Web_shaped": 227,
                       "source_observed": 8, "source_unknown": 91},
            "API_only_exclusions": copy.deepcopy(prerequisite["API_only_exclusions"]),
            "source_boundary_records": copy.deepcopy(prerequisite["source_boundary_records"]),
            "target_test_assessment": assessment,
            "rights_and_source_enrollment_prerequisites": rights,
            "current_enrollment_observation": enrollment, "acceptance_obligations": obligations,
            "switch_policy": {"enabled": False, "genuine_production_validator": None,
                              "all_six_production_obligations_accepted": False,
                              "inside_tuple_allowlist_before_qualification": "EXISTING_LOCAL_SOURCE_CALLER",
                              "outside_tuple_allowlist": "EXISTING_LOCAL_SOURCE_CALLER",
                              "nonstring_price": "SOURCE_SKIPS_TEXT_INTERPRETATION",
                              "raw_source_bytes_Option_presence_and_observed_Money": "PRESERVE_UNCHANGED",
                              "after_dispatch_pending_effect": "PRESERVE_UNKNOWN; NO_RESEND_RECOMPUTE_REFUND_OR_LOCAL_FALLBACK",
                              "known_unavailable": "PRESERVE_TYPED_FALSE_REASON; NO_SILENT_SUCCESS_SUBSTITUTION",
                              "before_dispatch_rollback": "LOCAL_ROUTE_REMAINS_INSTALLED",
                              "after_effect_rollback": "REQUIRES_GENUINE_EFFECT_RECONCILIATION"},
            "status": {"source_authority": "SOURCE_ONLY", "canonical_enrollment": "BLOCKED",
                       "target_exclusive_custody": None, "current_production_rights": None,
                       "production_Host_admission": None, "source_adapter_cutover": False,
                       "source_retirement": False, "all828_admitted_mappings": 0,
                       "production_obligations_closed": 0, "status": "SWITCH_PREPARED_DISABLED"}}


def inspect_contract(raw, expected_contract_sha256, root=ROOT, *, expected_target_sha256,
                     expected_rights_sha256):
    cohort.require_pin(raw, expected_contract_sha256)
    expected = build_contract(root, expected_target_sha256=expected_target_sha256,
                              expected_rights_sha256=expected_rights_sha256)
    same(candidate.strict_json(raw), expected, "caller selector, evidence, rights or production gate changed")
    return {"format": "ecdev-selected-price-switch-inspection-v1", "status": "VALID_PREPARED_SWITCH_DISABLED",
            "source_cases": 99, "distinct_request_tuples": 96, "production_obligations_closed": 0,
            "source_adapter_cutover": False, "current_production_rights": None}


def preview_offer(offer, root=ROOT, *, expected_target_sha256, expected_rights_sha256,
                  production_acceptance=None):
    if production_acceptance is not None:
        raise ValueError("production acceptance cannot be supplied: genuine validator is not implemented")
    if type(offer) is not dict:
        raise ValueError("original offer object required")
    contract = build_contract(root, expected_target_sha256=expected_target_sha256,
                               expected_rights_sha256=expected_rights_sha256)
    raw, hint = offer.get("price"), offer.get("priceCurrency")
    signature = None if type(raw) is not str else {
        "input_utf8_hex": raw.encode().hex(), "currency_hint": {"present": type(hint) is str,
                                                                 "value": hint if type(hint) is str else None},
        "decimal_separator": None, "digit_group_separator": None}
    matches = [row for row in contract["selector_groups"] if row["request_signature"] == signature]
    return {"format": "ecdev-selected-price-source-route-preview-v1", "original_offer": copy.deepcopy(offer),
            "selected_request_tuple": bool(matches),
            "source_case_indices": matches[0]["source_case_indices"] if matches else [],
            "text_interpretation_branch_entered": type(raw) is str,
            "route": "EXISTING_LOCAL_SOURCE_CALLER", "switch_enabled": False,
            "target_dispatch_performed": False, "derived_result_used_for_observed_money": False,
            "production_acceptance": "BLOCKED"}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--expect-target-sha256", required=True)
    parser.add_argument("--expect-rights-sha256", required=True)
    parser.add_argument("--inspect-contract", type=Path)
    parser.add_argument("--expect-contract-sha256")
    parser.add_argument("--preview-offer-json")
    args = parser.parse_args()
    if bool(args.inspect_contract) != bool(args.expect_contract_sha256):
        parser.error("contract input and independent SHA-256 required together")
    if args.inspect_contract and args.preview_offer_json is not None:
        parser.error("choose contract inspection or an offer preview")
    kwargs = {"expected_target_sha256": args.expect_target_sha256,
              "expected_rights_sha256": args.expect_rights_sha256}
    try:
        if args.inspect_contract:
            result = inspect_contract(candidate.regular(args.inspect_contract), args.expect_contract_sha256,
                                       args.source_root, **kwargs)
        elif args.preview_offer_json is not None:
            result = preview_offer(candidate.strict_json(args.preview_offer_json.encode()), args.source_root, **kwargs)
        else:
            result = build_contract(args.source_root, **kwargs)
        sys.stdout.buffer.write(cohort.encode(result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"selected caller switch refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
