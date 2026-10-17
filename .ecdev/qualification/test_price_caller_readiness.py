"""Actual source caller evidence must not become target execution or money authority."""

import copy
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

import price_caller_readiness as readiness


REPORT_PATH = readiness.ROOT / "research/commerce/governance/price-web-caller-acceptance-v1.json"
REPORT_SHA256 = "4a3c8b319133463772fae4a169a1d3e3ef68381404ed6d9debf6dcbdf3c86186"


class SourceCallerReadiness(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = REPORT_PATH.read_bytes()
        cls.report = json.loads(cls.raw)
        cls.value = readiness.build_readiness(cls.raw, REPORT_SHA256)
        cls.bytes = readiness.cohort.encode(cls.value)
        cls.pin = readiness.cohort.digest(cls.bytes)

    def check_report(self, report):
        raw = readiness.cohort.encode(report)
        return readiness.build_readiness(raw, readiness.cohort.digest(raw))

    def check_readiness(self, value):
        raw = readiness.cohort.encode(value)
        return readiness.inspect_readiness(self.raw, REPORT_SHA256, raw, readiness.cohort.digest(raw))

    def test_real_source_caller_cohort_has_distinct_complete_denominators(self):
        counts = self.value["counts"]
        self.assertEqual((counts["api_qualified"], counts["api_unmapped"], counts["corpus_total"]), (106, 2831, 2937))
        self.assertEqual((counts["web_qualified"], counts["web_unmapped"], counts["non_web_shape"]), (99, 2611, 227))
        self.assertEqual((counts["web_old_qualified"], counts["web_new_qualified"]), (94, 5))
        self.assertEqual([r["index"] for r in self.value["excluded_API_cases"]], [409, 1185, 1186, 1187, 1188, 1467, 1473])
        self.assertEqual(self.check_readiness(self.value)["source_web_cases"], 99)
        self.assertFalse(self.value["status"]["target_governed_caller_locally_run"])
        self.assertEqual(self.value["status"]["canonical_enrollment"], "BLOCKED")

    def test_empty_input_is_some_empty_and_none_input_cases_remain_excluded(self):
        proposed = {row["source_case_index"]: row for row in self.value["proposed_requests"]}
        for index in (1190, 1191, 1192, 1193):
            row = proposed[index]
            self.assertEqual(row["payload_utf8_hex"], "")
            self.assertEqual(row["payload_sha256"], readiness.cohort.digest(b""))
            self.assertFalse(row["input_is_none_fact_present"])
            self.assertTrue(row["source_Option_presence"]["input"])
            self.assertIsNone(row["EffectRef"])
            self.assertIsNone(row["resource_allocation"])
        self.assertEqual(proposed[1190]["text_selector_facts"], {})
        self.assertEqual(proposed[1193]["text_selector_facts"], {"currency_hint": "EUR"})
        self.assertNotIn(1188, proposed)
        empty_hint = next(r for r in self.report["negative_cases"] if r["name"] == "hint_empty_string")
        self.assertEqual(empty_hint["caller_hint_option"], {"present": True, "value": ""})

    def test_caller_pin_is_independent_mandatory_and_cannot_be_stale(self):
        for pin in (None, "", "0" * 64, "g" * 64, "sha256:" + REPORT_SHA256):
            with self.subTest(pin=pin), self.assertRaises(ValueError):
                readiness.build_readiness(self.raw, pin)
        with self.assertRaises(ValueError):
            readiness.build_readiness(self.raw + b" ", REPORT_SHA256)

    def test_denominator_shrink_and_false_execution_claims_are_refused_after_rebinding(self):
        for section, field, value in [("counts", "api_qualified", 99), ("counts", "api_unmapped", 2611),
                                      ("counts", "web_qualified", 106), ("counts", "web_shape", 99),
                                      (None, "target_execution_performed", True),
                                      (None, "target_acceptance_established", True),
                                      (None, "authority_granted", 0)]:
            changed = copy.deepcopy(self.report)
            (changed if section is None else changed[section])[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_missing_duplicate_or_reordered_original_case_and_exclusion_refuse(self):
        for mutation in ("missing", "duplicate", "reorder", "excluded"):
            changed = copy.deepcopy(self.report)
            if mutation == "missing":
                changed["cases"].pop()
            elif mutation == "duplicate":
                changed["cases"][-1] = changed["cases"][0]
            elif mutation == "reorder":
                changed["cases"].reverse()
            else:
                changed["excluded_cases"].pop()
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_raw_input_hint_presence_four_fields_float_and_observed_money_refuse_drift(self):
        for mutation in ("payload", "presence", "hint", "raw", "result", "float", "money", "ISO"):
            changed = copy.deepcopy(self.report)
            row = changed["cases"][0]
            if mutation == "payload":
                row["hypothetical_request"]["input_utf8"].append(32)
            elif mutation == "presence":
                row["source_request_presence"]["input"] = 1
            elif mutation == "hint":
                row["currency_hint"]["value"] = "USD"
            elif mutation == "raw":
                row["raw_json_ld"]["offers"]["price"] = "other"
            elif mutation == "result":
                row["interpretation"]["result"]["amount"] = "29"
            elif mutation == "float":
                row["interpretation"]["amount_float_bits"] = struct.pack(">d", 29.0).hex()
            elif mutation == "money":
                row["source_observed"]["product_price_minor"] = 2800
            else:
                row["interpretation"]["currency_token_is_iso_identity"] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_numeric_precision_nonstring_skip_empty_hint_and_resource_unknown_remain(self):
        for mutation in ("precision", "nonstring", "hint", "resource", "missing"):
            changed = copy.deepcopy(self.report)
            rows = {row["name"]: row for row in changed["negative_cases"]}
            if mutation == "precision":
                rows["numeric_lexeme_2980.0000000000001"]["raw_price_json"] = "2980.0"
            elif mutation == "nonstring":
                rows["null_price"]["interpretation"] = {}
            elif mutation == "hint":
                rows["hint_empty_string"]["caller_hint_option"] = {"present": False, "value": None}
            elif mutation == "resource":
                rows["price_over_4096_bytes"]["interpretation"]["state"] = "DERIVED"
            else:
                changed["negative_cases"].pop()
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_four_field_result_cannot_export_an_extra_field_after_report_rebinding(self):
        changed = copy.deepcopy(self.report)
        changed["cases"][0]["interpretation"]["result"]["extra"] = "injected"
        with self.assertRaises(ValueError):
            self.check_report(changed)

    def test_excluded_case_extra_authority_fields_cannot_be_copied_to_export(self):
        for field in ("target_acceptance_established", "authority_granted", "EffectRef"):
            changed = copy.deepcopy(self.report)
            changed["excluded_cases"][0][field] = True
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_boundary_money_resource_result_exact_lexeme_and_original_inputs_refuse_forgery(self):
        for mutation in ("numeric_money", "unknown_amount", "equal_numeric_lexeme", "changed_nonstring",
                         "changed_hint", "formatted_currency", "source_status", "numeric_raw", "extra_result"):
            changed = copy.deepcopy(self.report)
            rows = {row["name"]: row for row in changed["negative_cases"]}
            if mutation == "numeric_money":
                rows["numeric_lexeme_2.9e-1"]["source_observed"]["product_price_minor"] = 999999
            elif mutation == "unknown_amount":
                rows["price_over_4096_bytes"]["interpretation"]["result"].update(amount="12", amount_float=12.0)
            elif mutation == "equal_numeric_lexeme":
                rows["numeric_lexeme_2.98e+3"]["raw_price_json"] = "2980.0"
            elif mutation == "changed_nonstring":
                row = rows["boolean_price"]
                row["raw_price"] = False
                row["raw_price_json"] = "false"
                for key in ("source_json_ld", "raw_json_ld"):
                    row[key]["offers"]["price"] = False
            elif mutation == "changed_hint":
                rows["hint_null"]["raw_json_ld"]["offers"]["priceCurrency"] = ""
            elif mutation == "formatted_currency":
                rows["$12.99"]["interpretation"]["result"]["currency"] = "USD"
            elif mutation == "source_status":
                rows["null_price"]["source_observed"]["product_price_status"] = "OBSERVED"
            elif mutation == "numeric_raw":
                rows["numeric_lexeme_2.9e-1"]["raw_price"] = 29.0
            else:
                rows["free shipping"]["interpretation"]["result"]["extra"] = "injected"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_report(changed)

    def test_request_data_cannot_invent_effect_rights_budget_owner_or_cutover(self):
        for section, key, value in [("status", "accepted_owner", "invented"),
                                    ("status", "authority_grants", ["resource.work"]),
                                    ("status", "source_adapter_cutover", True),
                                    ("status", "target_governed_caller_locally_run", True),
                                    ("request", "EffectRef", {}), ("request", "resource_allocation", {"cpu_units": 1}),
                                    ("request", "input_is_none_fact_present", True)]:
            changed = copy.deepcopy(self.value)
            (changed["proposed_requests"][0] if section == "request" else changed[section])[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.check_readiness(changed)

    def test_cli_has_read_only_pinned_build_and_inspection(self):
        command = [sys.executable, "-B", str(Path(readiness.__file__)), "--report", str(REPORT_PATH),
                   "--expect-report-sha256", REPORT_SHA256]
        built = subprocess.run(command, capture_output=True, check=True)
        self.assertEqual(built.stdout, self.bytes)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "readiness.json")
            path.write_bytes(self.bytes)
            result = subprocess.run([*command, "--inspect-readiness", str(path), "--expect-readiness-sha256", self.pin],
                                    capture_output=True, check=True)
            self.assertEqual(json.loads(result.stdout)["status"], "VALID_SOURCE_CALLER_READINESS; TARGET_ADMISSION_PENDING")
            self.assertEqual(path.read_bytes(), self.bytes)
            self.assertEqual(list(Path(directory).iterdir()), [path])
            self.assertEqual(subprocess.run([*command, "--inspect-readiness", str(path)], capture_output=True).returncode, 2)


if __name__ == "__main__":
    unittest.main()
