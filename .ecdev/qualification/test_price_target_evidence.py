#!/usr/bin/env python3
"""Refuse receipt drift and production claims in actual published test-only evidence."""

import base64
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import price_cohort as cohort
import price_target_evidence as evidence


class TargetEvidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.kwargs = {"expected_target_sha256": evidence.TARGET_SHA256}
        cls.report_raw = evidence.candidate.regular(evidence.ROOT / evidence.TARGET_PATH)
        cls.report = evidence.candidate.strict_json(cls.report_raw)
        cls.contract = evidence.candidate.strict_json(evidence.candidate.regular(
            evidence.ROOT / evidence.PREREQUISITES_PATH))
        cls.corpus = evidence.candidate.strict_json(evidence.candidate.regular(
            evidence.ROOT / evidence.custody.manifest.CORPUS_PATH))
        cls.value = evidence.build_assessment(**cls.kwargs)
        cls.raw = cohort.encode(cls.value)
        cls.pin = cohort.digest(cls.raw)
        cls.parameters = evidence.wire_parameters(cls.report)

    def inspect(self, raw=None, pin=None):
        return evidence.inspect_assessment(self.raw if raw is None else raw,
                                           self.pin if pin is None else pin, **self.kwargs)

    def reject_assessment(self, mutate):
        changed = copy.deepcopy(self.value)
        mutate(changed)
        raw = cohort.encode(changed)
        with self.assertRaises(ValueError):
            self.inspect(raw, cohort.digest(raw))

    def replace_marker(self, report, old, new):
        command = report["command_receipts"]["test"]
        command["stdout_utf8"] = command["stdout_utf8"].replace(old, new)
        raw = command["stdout_utf8"].encode()
        command["stdout_sha256"] = cohort.digest(raw)
        report["raw_success_stdout_base64"] = base64.b64encode(raw).decode()
        report["raw_success_stdout_sha256"] = cohort.digest(raw)

    def altered_receipt(self, change):
        receipt = self.report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"]
        prefix, encoded = receipt.split(" ")
        body = bytearray(bytes.fromhex(encoded)[:-32])
        change(body)
        checksum = hashlib.sha256(self.parameters["checksum_domain"] + body).digest()
        return prefix + " " + (body + checksum).hex()

    def test_full_regeneration_reconciles_actual_source_files_and_109_frames(self):
        checked = self.inspect()
        self.assertEqual(checked["status"], "TEST_EVIDENCE_RECONCILED_ONLY")
        self.assertEqual(checked["wire_frames_reconciled"], 109)
        self.assertEqual(len(self.value["source_revision"]["source_files"]), 14)
        self.assertEqual(self.value["source_revision"]["commit"], evidence.SOURCE_COMMIT)
        self.assertEqual(self.value["source_revision"]["tree"], evidence.SOURCE_TREE)
        self.assertEqual(self.value["selected_source"]["witness_sha256"], evidence.WITNESS_SHA256)
        self.assertEqual(self.value["selected_source"]["prerequisites_sha256"], evidence.PREREQUISITES_SHA256)
        self.assertEqual(len(self.value["selected_source"]["bindings"]), 4)
        self.assertEqual(len(self.value["primary_receipts"]), 106)
        self.assertEqual(len(self.value["control_receipts"]), 3)
        self.assertEqual(self.value["source_observation_counts"], {"OBSERVED": 8, "UNKNOWN": 91})
        self.assertEqual(self.value["source_observed_indices"], evidence.OBSERVED)
        self.assertEqual([row["index"] for row in self.value["primary_receipts"][:99]],
                         self.contract["selected_web_indices"])
        self.assertEqual([row["index"] for row in self.value["primary_receipts"][99:]],
                         [row["index"] for row in self.contract["API_only_exclusions"]])

    def test_original_options_four_fields_and_float_bits_are_all_exact(self):
        for row in self.value["primary_receipts"]:
            case = self.corpus["cases"][row["index"]]
            result = row["decoded_receipt"]["result"]
            self.assertEqual(row["source_request"], evidence.caller.request(case))
            self.assertEqual(row["source_selector_facts"], evidence.selector_facts(case))
            self.assertEqual(row["payload_utf8_hex"], (case["input"] or "").encode().hex())
            self.assertEqual(result["fields"], case["expected"])
            self.assertEqual(result["amount_float_bits"], evidence.caller.float_bits(case["expected"]["amount_float"]))
            self.assertEqual(result["selectors_sha256"], evidence.selector_digest(evidence.selector_facts(case)))
            self.assertTrue(row["decoded_receipt"]["completed"])
        absent = {"currency_hint": None, "decimal_separator": None,
                  "digit_group_separator": None, "input": ""}
        empty_hint = dict(absent, currency_hint="")
        none_input = dict(absent, input=None)
        self.assertNotEqual(evidence.selector_digest(evidence.selector_facts(absent)),
                            evidence.selector_digest(evidence.selector_facts(empty_hint)))
        self.assertNotEqual(evidence.selector_digest(evidence.selector_facts(absent)),
                            evidence.selector_digest(evidence.selector_facts(none_input)))

    def test_exact_u64_sentinel_and_only_one_boundary_remain_typed_known_unavailable(self):
        row = self.value["control_receipts"][-1]
        self.assertIs(type(row["index"]), int)
        self.assertEqual(row["index"], 18446744073709551615)
        self.assertEqual(json.loads(cohort.encode(row))["index"], 18446744073709551615)
        self.assertEqual(row["decoded_receipt"]["result"]["unavailable_code"], 8)
        self.assertIs(row["decoded_receipt"]["completed"], False)
        boundary = self.value["boundary_execution"]
        self.assertEqual((boundary["executed"], boundary["original_source_boundaries"],
                          boundary["remaining_boundaries_not_claimed_executed"]), (1, 20, 19))
        for index in (float(evidence.U64_MAX), evidence.U64_MAX + 1, True, -1):
            report = copy.deepcopy(self.report)
            report["supplemental_control_receipts"][-1]["index"] = index
            with self.subTest(index=index), self.assertRaises(ValueError):
                evidence.reconcile_rows(report, self.contract, self.corpus)

    def test_all_command_hashes_initial_failures_and_lint_blockers_are_preserved(self):
        self.assertEqual(evidence.check_commands(self.report), base64.b64decode(self.report["raw_success_stdout_base64"]))
        validation = self.value["command_validation"]
        self.assertEqual(validation["targeted_tests"], {"passed": 5, "failed": 0, "ignored": 0})
        self.assertEqual(validation["normal_tests"], {"passed": 14, "failed": 0, "ignored": 6})
        self.assertEqual(validation["initial_attempt"], {"passed": 3, "failed": 2, "retained": True})
        self.assertEqual(validation["strict_clippy"], "BLOCKED_INHERITED_LINT")
        self.assertEqual(validation["no_deps_strict_clippy"], "BLOCKED_INHERITED_LINT")
        self.assertEqual(validation["single_lint_filtered_diagnostic"], "EXIT_ZERO_FILTERED_DIAGNOSTIC_ONLY")
        self.assertFalse(self.value["exact_published_revision_binary_attestation"])
        self.assertTrue(self.value["runtime_provenance"]["working_tree_not_pristine"])
        self.assertNotEqual(self.value["runtime_provenance"]["base_head"], evidence.TARGET_PUBLICATION)
        for mutate in (lambda r: r.update(raw_success_stdout_base64="!"),
                       lambda r: r.update(raw_success_stdout_sha256="0" * 64),
                       lambda r: r["command_receipts"]["test"].update(stdout_utf8="changed"),
                       lambda r: r["command_receipts"]["scoped-clippy"].update(exit_status=0)):
            report = copy.deepcopy(self.report)
            mutate(report)
            with self.assertRaises(ValueError):
                evidence.check_commands(report)

    def test_receipt_checksum_case_bounds_version_tags_and_trailing_bytes_are_refused(self):
        receipt = self.report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"]
        corrupt = receipt[:-1] + ("0" if receipt[-1] != "0" else "1")
        for changed in (corrupt, receipt.upper(), receipt + "00", receipt.partition(" ")[0] + " " + "00" * 4097,
                        self.altered_receipt(lambda body: body.__setitem__(0, 2)),
                        self.altered_receipt(lambda body: body.extend(b"\0"))):
            with self.subTest(length=len(changed)), self.assertRaises(ValueError):
                evidence.decode_receipt(changed, self.parameters)
        result = bytes.fromhex(self.value["primary_receipts"][0]["decoded_receipt"]["result_hex"])
        for changed in (result + b"\0", result[:1], bytes(513)):
            with self.assertRaises(ValueError):
                evidence.decode_result(changed)

    def test_float_bits_mutation_with_new_checksum_and_matching_stdout_is_refused(self):
        # Corrupt one finite advisory bit while preserving all outer integrity framing.
        original = self.report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"]
        changed = self.altered_receipt(lambda body: body.__setitem__(-1, body[-1] ^ 1))
        report = copy.deepcopy(self.report)
        report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"] = changed
        self.replace_marker(report, original, changed)
        with self.assertRaises(ValueError):
            evidence.reconcile_rows(report, self.contract, self.corpus)

    def test_receipt_context_native_request_and_ir_joins_are_required(self):
        for field in ("input_sha256", "ir_sha256", "request_sha256"):
            report = copy.deepcopy(self.report)
            row = report["primary_ordered106_receipts"][0]
            old = row[field]
            row[field] = "0" * 64
            self.replace_marker(report, old, row[field])
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.reconcile_rows(report, self.contract, self.corpus)
        original = self.report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"]
        # The actual first lease is a unique vector; same-length mutation retains valid frame sizes.
        def change_lease(body):
            needle = b"ecdev362-45.0.lease"
            offset = body.index(needle)
            body[offset:offset + len(needle)] = needle.replace(b"45", b"46")
        changed = self.altered_receipt(change_lease)
        report = copy.deepcopy(self.report)
        report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"] = changed
        self.replace_marker(report, original, changed)
        with self.assertRaises(ValueError):
            evidence.reconcile_rows(report, self.contract, self.corpus)

    def test_drop_order_index_and_original_selector_drift_are_refused(self):
        for mutate in (lambda r: r["primary_ordered106_receipts"].pop(),
                       lambda r: r["primary_ordered106_receipts"].reverse(),
                       lambda r: r["primary_ordered106_receipts"][0].update(index=409),
                       lambda r: r["supplemental_control_receipts"].reverse()):
            report = copy.deepcopy(self.report)
            mutate(report)
            with self.assertRaises(ValueError):
                evidence.reconcile_rows(report, self.contract, self.corpus)
        corpus = copy.deepcopy(self.corpus)
        corpus["cases"][45]["currency_hint"] = None
        with self.assertRaisesRegex(ValueError, "selector Option"):
            evidence.reconcile_rows(self.report, self.contract, corpus)
        corpus["cases"][45]["currency_hint"] = "$"
        corpus["cases"][45]["input"] = "29"
        with self.assertRaisesRegex(ValueError, "payload digest"):
            evidence.reconcile_rows(self.report, self.contract, corpus)

    def test_original_status_and_all_production_obligations_remain_open(self):
        self.assertEqual(self.value["original_source_status"], self.contract["status"])
        self.assertEqual(self.value["original_six_obligation_evidence"],
                         {row["key"]: None for row in self.contract["obligations"]})
        for key, obligation in self.value["obligation_assessment"].items():
            self.assertEqual(obligation["production_status"], "OPEN")
            self.assertIsNone(obligation["original_evidence"])
            self.assertEqual(obligation["test_context_subevidence"] is not None,
                             key in ("target_execution", "authority", "unknown_effect"))
        self.assertFalse(self.value["production_status"]["production_qualification_established"])
        changes = [lambda v: v["production_status"].update(authority_granted=True),
                   lambda v: v["original_source_status"].update(source_adapter_cutover=True),
                   lambda v: v["original_six_obligation_evidence"].update(target_execution={}),
                   lambda v: v["obligation_assessment"]["authority"].update(production_status="CLOSED"),
                   lambda v: v.update(exact_published_revision_binary_attestation=True),
                   lambda v: v["boundary_execution"].update(executed=20),
                   lambda v: v["control_receipts"][-1].update(index=float(evidence.U64_MAX))]
        for mutation in changes:
            self.reject_assessment(mutation)

    def test_independent_fixed_pins_and_all_selected_source_members_are_required(self):
        with self.assertRaises(ValueError):
            evidence.build_assessment(expected_target_sha256="0" * 64)
        with self.assertRaises(ValueError):
            self.inspect(pin="0" * 64)
        original_member = evidence.custody.committed_file

        def wrong_blob(root, entries, path, *args, **kwargs):
            row = original_member(root, entries, path, *args, **kwargs)
            if path == self.report["source_files"][0]["source_path"]:
                row["blob"] = "0" * 40
            return row

        with mock.patch.object(evidence.custody, "committed_file", side_effect=wrong_blob), self.assertRaises(ValueError):
            evidence.build_assessment(**self.kwargs)
        original_read = evidence.candidate.regular

        def changed_current_material(path):
            raw = original_read(path)
            return raw + b"\n" if Path(path) == evidence.ROOT / evidence.caller.CALLER_PATH else raw

        with mock.patch.object(evidence.candidate, "regular", side_effect=changed_current_material), self.assertRaises(ValueError):
            evidence.build_assessment(**self.kwargs)
        duplicate = b'{"status":"x","status":"y"}'
        with self.assertRaises(ValueError):
            self.inspect(duplicate, cohort.digest(duplicate))

    def test_cli_roundtrip_outputs_only_test_assessment_and_does_not_rewrite_inputs(self):
        command = [sys.executable, "-B", str(Path(evidence.__file__)),
                   "--expect-target-sha256", evidence.TARGET_SHA256]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "assessment.json")
            path.write_bytes(self.raw)
            result = subprocess.run([*command, "--inspect-assessment", str(path),
                                     "--expect-assessment-sha256", self.pin], check=True, capture_output=True)
            self.assertEqual(json.loads(result.stdout)["status"], "TEST_EVIDENCE_RECONCILED_ONLY")
            refused = subprocess.run([*command, "--inspect-assessment", str(path)], capture_output=True)
            self.assertEqual(refused.returncode, 2)
            self.assertFalse(refused.stdout)
            self.assertEqual(path.read_bytes(), self.raw)
            self.assertEqual(list(Path(directory).iterdir()), [path])
        self.assertEqual(evidence.candidate.regular(evidence.ROOT / evidence.TARGET_PATH), self.report_raw)


if __name__ == "__main__":
    unittest.main()
