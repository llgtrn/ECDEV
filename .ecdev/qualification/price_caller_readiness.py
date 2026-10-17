#!/usr/bin/env python3
"""Inspect source Web caller evidence and export request data, without target admission."""

import argparse
import copy
from decimal import Decimal, InvalidOperation
import json
import math
from pathlib import Path
import struct
import sys

import price_candidate as candidate
import price_cohort as cohort
import price_manifest as manifest


ROOT = Path(__file__).absolute().parents[2]
COHORT_PATH = "research/commerce/governance/source-price-cohort-db92bf27-v1.json"
COHORT_SHA256 = "f7b2d7b57115402e26818e76aa765674703001d48105712feb4116097d690d66"
CALLER_PATH = "adapter/web/src/lib.rs"
CALLER_SHA256 = "34539b4835e6a66bd2d091de1f7c7f6f543e7200b8c5920660cfdb5562a9e944"
PRICE_PATH = "adapter/web/src/price.rs"
PRICE_SHA256 = "5aac2cf7a43e3d2fad31e233d1abdbb97d21197cf245cbc1f49d0901fecd8ec3"
FIELDS = ("input", "currency_hint", "decimal_separator", "digit_group_separator")
CHECKS = {"bounded_regular_frozen_inputs_before_after", "currency_token_not_iso_identity",
          "derived_not_observed_money_authority", "full_api_totals_preserved", "hint_option_presence_preserved",
          "nullable_finite_float_bits_exact", "original_corpus_four_fields", "qualified_web_cohort_exact",
          "raw_json_ld_and_price_preserved"}
NEGATIVES = {"null_price", "boolean_price", "array_price", "object_price", "missing_price",
             "numeric_lexeme_2980.0000000000001", "numeric_lexeme_0.29000000000000001",
             "numeric_lexeme_9007199254740993.0", "numeric_lexeme_2.98e+3", "numeric_lexeme_2.9e-1",
             "$12.99", "free shipping", "hint_absent", "hint_null", "hint_nonstring", "hint_empty_string",
             "hint_currency_string", "empty_string_price_and_hint", "unicode_and_embedded_closing_script",
             "price_over_4096_bytes"}


def equal(actual, expected, detail):
    if candidate.canonical(actual) != candidate.canonical(expected):
        raise ValueError(detail)


def request(case):
    return {key: case[key] for key in FIELDS}


def presence(value):
    return {key: value[key] is not None for key in FIELDS}


def float_bits(value):
    if value is None:
        return None
    if type(value) not in (float, int) or not math.isfinite(value):
        raise ValueError("finite advisory float required")
    return struct.pack(">d", value).hex()


def source_money(raw, hint):
    """Independent narrow check for these USD/EUR/$/absent bounded source cases."""
    if hint not in ("USD", "EUR") or type(raw) is not str:
        return None
    whole, _, fraction = raw.partition(".")
    fraction = fraction.rstrip("0")
    if (not whole or not whole.isascii() or not whole.isdigit()
            or (fraction and (not fraction.isascii() or not fraction.isdigit())) or len(fraction) > 2):
        return None
    # The source parses the unscaled whole part as signed i64 before scaling.
    value = int(whole) * 100 + (int(fraction) * 10 ** (2 - len(fraction)) if fraction else 0)
    return value if int(whole) <= 2**63 - 1 and value <= 2**63 - 1 else None


def load_context(root):
    inputs, raw_corpus = cohort.load_inputs(root)
    pins = {role: pin for role, (_, pin) in cohort.ARTIFACTS.items()}
    selected = cohort.build_cohort(inputs, pins, raw_corpus)
    frozen = candidate.regular(root / COHORT_PATH)
    cohort.require_pin(frozen, COHORT_SHA256)
    equal(candidate.strict_json(frozen), selected, "frozen cohort changed")
    corpus = manifest._pinned_json(raw_corpus, manifest.CORPUS_SHA256, "corpus")
    for path, pin in ((CALLER_PATH, CALLER_SHA256), (PRICE_PATH, PRICE_SHA256)):
        cohort.require_pin(candidate.regular(root / path), pin)
    return inputs, raw_corpus, selected, corpus


def inspect_report(raw, expected_report_sha256, root=ROOT):
    root = candidate.physical(root)
    cohort.require_pin(raw, expected_report_sha256)
    inputs, raw_corpus, selected, corpus = load_context(root)
    report = candidate.strict_json(raw)
    if report["format"] != "ECDEV_PRICE_WEB_CALLER_V1" or report["status"] != "PASS":
        raise ValueError("passing source caller report required")
    equal(report["entry"], "ecdev_web::extract -> products -> price::parse_price", "source entry changed")
    equal(report["execution_mode"], "OFFLINE_SYNTHETIC_JSON_LD_ACTUAL_SOURCE_CALLER", "source scope changed")
    for key in ("target_execution_performed", "target_acceptance_established", "authority_granted"):
        equal(report[key], False, "source caller evidence cannot grant target authority or execution")
    equal(report["checks"]["status"], "PASS", "caller checks failed")
    if type(report["checks"]["assertion_count"]) is not int or report["checks"]["assertion_count"] < 1:
        raise ValueError("actual source assertions required")
    for key in CHECKS:
        equal(report["checks"][key], True, "source caller preservation check missing or failed")
    expected_inputs = [(manifest.CORPUS_PATH, raw_corpus), (manifest.SIDECAR_PATH, inputs["price_sidecar"]),
                       (CALLER_PATH, candidate.regular(root / CALLER_PATH)),
                       (PRICE_PATH, candidate.regular(root / PRICE_PATH))]
    equal(report["frozen_inputs"], [{"path": p, "sha256": cohort.digest(b), "size_bytes": len(b),
                                      "verified_before": True, "verified_after": True}
                                     for p, b in expected_inputs], "source report input binding changed")
    cases = corpus["cases"]
    api = selected["coverage"]["qualified_case_indices"]
    shape = {i for i, case in enumerate(cases) if type(case["input"]) is str
             and case["decimal_separator"] is None and case["digit_group_separator"] is None}
    eligible = [i for i in api if i in shape]
    excluded = [i for i in api if i not in shape]
    additions = set(selected["coverage"]["new_candidate_indices"])
    counts = {"corpus_total": len(cases), "api_qualified": len(api), "api_unmapped": len(cases) - len(api),
              "web_shape": len(shape), "non_web_shape": len(cases) - len(shape),
              "web_qualified": len(eligible), "web_unmapped": len(shape - set(api)),
              "qualified_web_excluded": len(excluded), "web_old_qualified": len(set(eligible) - additions),
              "web_new_qualified": len(set(eligible) & additions), "negative_cases": 20}
    equal(counts, {"corpus_total": 2937, "api_qualified": 106, "api_unmapped": 2831, "web_shape": 2710,
                   "non_web_shape": 227, "web_qualified": 99, "web_unmapped": 2611,
                   "qualified_web_excluded": 7, "web_old_qualified": 94, "web_new_qualified": 5, "negative_cases": 20},
          "bounded caller cohort changed")
    equal(report["counts"], counts, "caller counts cannot replace the complete API denominator")
    equal([row["index"] for row in report["cases"]], eligible, "duplicate, omitted or reordered caller cases")
    for row, index in zip(report["cases"], eligible):
        case = cases[index]
        expected_request = request(case)
        hint = {"present": case["currency_hint"] is not None, "value": case["currency_hint"]}
        equal(row["name"], case["name"], "source case name changed")
        equal(row["source_request"], expected_request, "original source selectors changed")
        equal(row["source_request_presence"], presence(expected_request), "selector Option presence changed")
        equal(row["raw_price"], case["input"], "original raw price changed")
        equal(row["currency_hint"], hint, "raw hint presence or value changed")
        offer = {"price": case["input"]}
        if hint["present"]:
            offer["priceCurrency"] = hint["value"]
        equal(row["raw_json_ld"], {"@context": "https://schema.org", "@type": "Product",
                                     "name": "Bounded offline acceptance", "offers": offer},
              "original JSON-LD evidence changed")
        equal(sorted(row["interpretation"]["result"]), sorted(case["expected"]),
              "source result must contain exactly the original four fields")
        for key in ("amount", "currency", "amount_text"):
            equal(row["interpretation"]["result"][key], case["expected"][key], "source four-field result differs")
        bits = float_bits(case["expected"]["amount_float"])
        equal(float_bits(row["interpretation"]["result"]["amount_float"]), bits, "source float bits differ")
        equal(row["interpretation"]["amount_float_bits"], bits, "recorded source float bits differ")
        for key in ("used_for_observed_price", "currency_token_is_iso_identity"):
            equal(row["interpretation"][key], False, "derived values cannot become observed money or ISO identity")
        equal(row["interpretation"]["state"], "DERIVED", "qualified source case state changed")
        observed = source_money(case["input"], case["currency_hint"])
        equal(row["source_observed"], {"offer_price_minor": observed, "product_price_minor": observed,
                                       "product_price_status": "OBSERVED" if observed is not None else "UNKNOWN"},
              "original source observed money changed")
        equal(row["hypothetical_request"], {"input_utf8": list(case["input"].encode()), "currency_hint": hint,
                                             "decimal_separator": None, "digit_group_separator": None},
              "hypothetical request does not preserve original UTF-8 or selector presence")
        equal(row["actual_source_caller_invoked"], True, "source caller was not invoked")
        for key in ("target_execution_performed", "target_acceptance_established"):
            equal(row[key], False, "source evidence cannot claim target execution")
    equal([row["index"] for row in report["excluded_cases"]], excluded, "excluded API cases must stay visible")
    for row, index in zip(report["excluded_cases"], excluded):
        case = cases[index]
        equal(row, {"index": index, "name": case["name"], "source_request": request(case),
                    "source_request_presence": presence(request(case)),
                    "reason": "INPUT_NONE_STRING_PRICE_REQUIRED" if case["input"] is None
                    else "EXPLICIT_SELECTORS_UNAVAILABLE_TO_WEB_CALLER",
                    "actual_source_caller_invoked": False, "target_execution_performed": False},
              "excluded source identity, selectors, scope or exact row fields changed")
    inspect_negatives(report["negative_cases"])
    return report, selected, cases


def boundary_specs():
    """Fixed inputs and source outcomes, independent of the supplied report."""
    specs = {}

    def add(name, offer, money=None, result=None, state="DERIVED", literal=None):
        raw = offer.get("price")
        node = {"@context": "https://schema.org", "@type": "Product",
                "name": "Bounded offline acceptance", "offers": offer}
        interpretation = None if result is None else {
            "state": state, "result": dict(zip(("amount", "currency", "amount_text", "amount_float"), result)),
            "method": "FIRST_PRICE_TEXT_CURRENCY_TOKEN_AND_SEPARATOR_HEURISTICS",
            "used_for_observed_price": False, "currency_token_is_iso_identity": False,
            "limitations": "First number may precede the actual price; signs may be discarded; free substring "
                           "may become zero. Requires source review before commercial use."}
        specs[name] = {"name": name, "status": "PASS", "source_json_ld": node, "raw_json_ld": node,
                       "raw_price": raw, "raw_price_json": literal if literal is not None else
                       json.dumps(raw, ensure_ascii=False, separators=(",", ":")),
                       "price_field_present": "price" in offer,
                       "currency_hint": {"present": "priceCurrency" in offer, "value": offer.get("priceCurrency")},
                       "source_observed": {"offer_price_minor": money, "product_price_minor": money,
                                           "product_price_status": "UNKNOWN" if money is None else "OBSERVED"},
                       "interpretation": interpretation, "target_execution_performed": False,
                       "target_acceptance_established": False}

    for name, value in (("null_price", None), ("boolean_price", True), ("array_price", ["12.99"]),
                        ("object_price", {"value": "12.99"})):
        add(name, {"price": value, "priceCurrency": "USD"})
    add("missing_price", {"priceCurrency": "USD"})
    for literal, currency, money in (("2980.0000000000001", "JPY", None),
                                     ("0.29000000000000001", "USD", None),
                                     ("9007199254740993.0", "JPY", 9007199254740993),
                                     ("2.98e+3", "JPY", 2980), ("2.9e-1", "USD", 29)):
        # Decoded numeric projections follow the JSON reader; the separate literal
        # remains exact, including exponent spelling and all precision digits.
        add("numeric_lexeme_" + literal, {"price": json.loads(literal), "priceCurrency": currency},
            money=money, literal=literal)
    add("$12.99", {"price": "$12.99", "priceCurrency": "USD"}, result=("12.99", "$", "12.99", 12.99))
    add("free shipping", {"price": "free shipping"}, result=("0", None, "0", 0.0))
    for name, hint in (("hint_absent", None), ("hint_null", None), ("hint_nonstring", 42),
                       ("hint_empty_string", ""), ("hint_currency_string", "EUR")):
        offer = {"price": "12.99"}
        if name != "hint_absent":
            offer["priceCurrency"] = hint
        add(name, offer, money=1299 if hint == "EUR" else None,
            result=("12.99", "EUR" if hint == "EUR" else None, "12.99", 12.99))
        specs[name]["caller_hint_option"] = {"present": type(hint) is str,
                                             "value": hint if type(hint) is str else None}
    add("empty_string_price_and_hint", {"price": "", "priceCurrency": ""}, result=(None, None, None, None))
    add("unicode_and_embedded_closing_script",
        {"price": "€12.99 </script><script>inert & 雪</script>", "priceCurrency": '\"<&雪>'},
        result=("12.99", "€", "12.99", 12.99))
    add("price_over_4096_bytes", {"price": "$" * 4097, "priceCurrency": "USD"},
        result=(None, None, None, None), state="UNKNOWN")
    return specs


def inspect_negatives(rows):
    if len(rows) != 20 or {row["name"] for row in rows} != NEGATIVES:
        raise ValueError("all twenty caller boundary negatives required")
    expected = boundary_specs()
    for row in rows:
        equal(row, expected[row["name"]], "boundary input, exact literal, result or observed money changed")
        if row["name"].startswith("numeric_lexeme_"):
            literal = row["name"].removeprefix("numeric_lexeme_")
            try:
                actual = Decimal(row["raw_price_json"])
            except (InvalidOperation, TypeError) as error:
                raise ValueError("numeric source literal invalid") from error
            if not actual.is_finite() or actual != Decimal(literal):
                raise ValueError("numeric source precision changed")


def build_readiness(raw, expected_report_sha256, root=ROOT):
    report, selected, _ = inspect_report(raw, expected_report_sha256, root)
    target = selected["bounded_price_reference"]["proposed_target"]
    proposed = []
    for row in report["cases"]:
        request_data = row["source_request"]
        payload = request_data["input"].encode()
        facts = {} if request_data["currency_hint"] is None else {"currency_hint": request_data["currency_hint"]}
        proposed.append({"source_case_index": row["index"], "source_case_name": row["name"],
                         "payload_utf8_hex": payload.hex(), "payload_sha256": cohort.digest(payload),
                         "text_selector_facts": facts, "input_is_none_fact_present": False,
                         "source_Option_presence": copy.deepcopy(row["source_request_presence"]),
                         "source_four_field_result": copy.deepcopy(row["interpretation"]["result"]),
                         "source_float_bits": row["interpretation"]["amount_float_bits"],
                         "full_IR_digest": None, "EffectRef": None, "requester_authentication": None,
                         "resource_allocation": None, "current_rights": None, "admission": "PENDING"})
    return {"format": "ecdev-price-web-caller-readiness-v1",
            "scope": "RECORDED_SOURCE_CALLER_ACCEPTANCE_AND_PROPOSED_TARGET_REQUEST_DATA; NO_TARGET_EXECUTION",
            "source_report_sha256": cohort.digest(raw), "frozen_cohort_sha256": COHORT_SHA256,
            "counts": copy.deepcopy(report["counts"]),
            "caller_scope_note": "These99 are94 prior plus5 EUR additions, not the separate historical old99 cohort",
            "actual_source_entry": "ecdev_web::extract -> products -> price::parse_price(Some(raw),hint,None,None)",
            "target_reference": copy.deepcopy(target), "proposed_requests": proposed,
            "excluded_API_cases": copy.deepcopy(report["excluded_cases"]),
            "remaining_requirements": {
                "C2": "Synthetic offline public source function executed; target native production admission NOT_RUN",
                "C3": "Source report inputs locally verified; target exact-revision execution NOT_RUN",
                "C4": "Source caller cutover NOT_PERFORMED; all unmapped and excluded cases preserved",
                "C5": "Target owner/runtime authority admission NOT_ESTABLISHED"},
            "status": {"source_authority": "SOURCE_ONLY", "canonical_enrollment": "BLOCKED",
                       "canonical_assignment": None, "accepted_owner": None, "authority": None,
                       "authority_grants": [], "current_rights": None, "world_admission": None,
                       "admission": "PENDING", "source_adapter_cutover": False, "source_retirement": False,
                       "target_governed_caller_locally_run": False, "native_v1": "INCOMPLETE"}}


def inspect_readiness(raw, expected_report_sha256, readiness_bytes, expected_readiness_sha256, root=ROOT):
    cohort.require_pin(readiness_bytes, expected_readiness_sha256)
    expected = build_readiness(raw, expected_report_sha256, root)
    equal(candidate.strict_json(readiness_bytes), expected,
          "readiness request data, unresolved identity or pending authority changed")
    return {"format": "ecdev-price-web-caller-readiness-inspection-v1",
            "status": "VALID_SOURCE_CALLER_READINESS; TARGET_ADMISSION_PENDING",
            "readiness_sha256": cohort.digest(readiness_bytes), "source_web_cases": 99,
            "API_qualified": 106, "API_unmapped": 2831, "target_execution_performed": False,
            "canonical_enrollment": "BLOCKED", "canonical_assignment": None}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--report", required=True, type=Path)
    parser.add_argument("--expect-report-sha256", required=True)
    parser.add_argument("--inspect-readiness", type=Path)
    parser.add_argument("--expect-readiness-sha256")
    args = parser.parse_args()
    if bool(args.inspect_readiness) != bool(args.expect_readiness_sha256):
        parser.error("readiness input and independent readiness pin are required together")
    try:
        raw = candidate.regular(args.report)
        result = (inspect_readiness(raw, args.expect_report_sha256, candidate.regular(args.inspect_readiness),
                                    args.expect_readiness_sha256, args.source_root) if args.inspect_readiness
                  else build_readiness(raw, args.expect_report_sha256, args.source_root))
        sys.stdout.buffer.write(cohort.encode(result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"caller readiness refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
