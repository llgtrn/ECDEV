"""Test Host evidence and copyright notices must never activate the source switch."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import price_selected_switch as switch


PINS = {"expected_target_sha256": switch.target.TARGET_SHA256, "expected_rights_sha256": switch.RIGHTS_SHA256}


class SelectedCallerSwitch(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.value = switch.build_contract(**PINS)
        cls.raw = switch.cohort.encode(cls.value)
        cls.pin = switch.cohort.digest(cls.raw)

    def check_changed(self, value):
        raw = switch.cohort.encode(value)
        return switch.inspect_contract(raw, switch.cohort.digest(raw), **PINS)

    def test_exact_tuple_allowlist_preserves_all99_source_identities_and_duplicates(self):
        groups = self.value["selector_groups"]
        self.assertEqual(len(groups), 96)
        self.assertEqual(sorted(i for row in groups for i in row["source_case_indices"]),
                         self.value["selected_case_indices"])
        self.assertEqual([g["source_case_indices"] for g in groups if len(g["source_case_indices"]) > 1],
                         [[627, 799], [832, 926], [2170, 2914]])
        self.assertEqual([r["index"] for r in self.value["API_only_exclusions"]],
                         [409, 1185, 1186, 1187, 1188, 1467, 1473])
        self.assertTrue(all(not row["input_is_none_fact_present"] for row in groups))

    def test_six_production_gates_open_despite_test_success_and_source_copyright_closures(self):
        self.assertEqual([r["key"] for r in self.value["acceptance_obligations"]], list(switch.OBLIGATIONS))
        for row in self.value["acceptance_obligations"]:
            self.assertEqual(row["production_acceptance"], "OPEN")
            self.assertIsNone(row["original_evidence"])
        rights = self.value["rights_and_source_enrollment_prerequisites"]
        self.assertEqual(rights["source_owned_closures"]["status"],
                         "BOUNDED_SOURCE_NOTICE_AND_PROVENANCE_PREREQUISITES_DOCUMENTED")
        self.assertIsNone(rights["status"]["current_production_rights"])
        self.assertFalse(self.value["switch_policy"]["enabled"])
        self.assertEqual(self.value["status"]["production_obligations_closed"], 0)
        observation = self.value["current_enrollment_observation"]
        self.assertTrue(observation["published_engine_target_locks_byte_equal"])
        self.assertFalse(observation["source_registered"])
        self.assertEqual(len(observation["registered_shards"]), 7)

    def test_true_money_observations_and_known_unknown_effect_states_remain_distinct(self):
        assessment = self.value["target_test_assessment"]
        self.assertEqual(assessment["source_observation_counts"], {"OBSERVED": 8, "UNKNOWN": 91})
        self.assertEqual(assessment["boundary_execution"]["executed"], 1)
        self.assertEqual(assessment["boundary_execution"]["original_source_boundaries"], 20)
        self.assertEqual(assessment["boundary_execution"]["control_index"], 2**64 - 1)
        self.assertIn("NO_RESEND_RECOMPUTE_REFUND_OR_LOCAL_FALLBACK",
                      self.value["switch_policy"]["after_dispatch_pending_effect"])
        self.assertIn("NO_SILENT_SUCCESS_SUBSTITUTION", self.value["switch_policy"]["known_unavailable"])
        self.assertEqual(self.value["counts"]["full_API_total"], 2937)
        self.assertEqual(self.value["counts"]["full_API_unmapped"], 2831)

    def test_source_caller_chain_and_schema_UI_consumers_are_byte_bound(self):
        paths = {r["path"] for r in self.value["real_source_caller_files"]}
        self.assertEqual(paths, set(switch.CALLER_PATHS))
        for member in self.value["real_source_caller_files"]:
            raw = switch.candidate.regular(switch.ROOT / member["path"])
            self.assertEqual(switch.cohort.digest(raw), member["sha256"])
            self.assertEqual(len(raw), member["bytes"])

    def test_preview_is_local_for_eligible_some_empty_and_unsupported_source_offers(self):
        for offer, eligible, entered, indices in [
                ({"price": "", "priceCurrency": "EUR"}, True, True, [1193]),
                ({"price": None, "priceCurrency": "EUR"}, False, False, []),
                ({"price": 1299, "priceCurrency": "EUR"}, False, False, []),
                ({"price": "$12.99", "priceCurrency": "USD", "source_case_index": 45}, False, True, [])]:
            result = switch.preview_offer(offer, **PINS)
            with self.subTest(offer=offer):
                self.assertEqual(result["original_offer"], offer)
                self.assertEqual(result["selected_request_tuple"], eligible)
                self.assertEqual(result["text_interpretation_branch_entered"], entered)
                self.assertEqual(result["source_case_indices"], indices)
                self.assertEqual(result["route"], "EXISTING_LOCAL_SOURCE_CALLER")
                self.assertFalse(result["target_dispatch_performed"])
                self.assertFalse(result["derived_result_used_for_observed_money"])

    def test_hint_none_and_some_empty_do_not_collapse(self):
        absent = switch.preview_offer({"price": "67"}, **PINS)
        empty = switch.preview_offer({"price": "67", "priceCurrency": ""}, **PINS)
        self.assertIn(415, absent["source_case_indices"])
        self.assertFalse(empty["selected_request_tuple"])
        self.assertEqual(empty["original_offer"]["priceCurrency"], "")

    def test_self_granted_production_acceptance_is_rejected(self):
        for fake in ({}, False, {"authority": "test", "exclusive_custody": True, "registry": True}):
            with self.subTest(fake=fake), self.assertRaises(ValueError):
                switch.preview_offer({"price": ""}, production_acceptance=fake, **PINS)

    def test_rebound_contract_cannot_expand_scope_or_close_any_gate(self):
        for mutation in ("enabled", "rights", "gates", "custody", "registry", "selectors", "duplicates",
                         "unknown", "typed_false", "extra"):
            changed = copy.deepcopy(self.value)
            if mutation == "enabled":
                changed["switch_policy"]["enabled"] = True
            elif mutation == "rights":
                changed["status"]["current_production_rights"] = {}
            elif mutation == "gates":
                changed["acceptance_obligations"][0]["production_acceptance"] = "CLOSED"
            elif mutation == "custody":
                changed["status"]["target_exclusive_custody"] = True
            elif mutation == "registry":
                changed["current_enrollment_observation"]["source_registered"] = True
            elif mutation == "selectors":
                changed["selector_groups"][0]["request_signature"]["currency_hint"]["present"] = False
            elif mutation == "duplicates":
                group = next(r for r in changed["selector_groups"] if len(r["source_case_indices"]) > 1)
                group["source_case_indices"].pop()
            elif mutation == "unknown":
                changed["switch_policy"]["after_dispatch_pending_effect"] = "RETRY_LOCAL"
            elif mutation == "typed_false":
                changed["switch_policy"]["enabled"] = 0
            else:
                changed["authority_granted"] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_changed(changed)

    def test_independent_input_and_contract_pins_are_required(self):
        for key in PINS:
            for bad in (None, "", "0" * 64):
                kwargs = dict(PINS, **{key: bad})
                with self.subTest(key=key, pin=bad), self.assertRaises(ValueError):
                    switch.build_contract(**kwargs)
        self.assertEqual(switch.inspect_contract(self.raw, self.pin, **PINS)["production_obligations_closed"], 0)
        with self.assertRaises(ValueError):
            switch.inspect_contract(self.raw + b" ", self.pin, **PINS)

    def test_cli_build_and_inspection_do_not_change_input_files(self):
        command = [sys.executable, "-B", str(Path(switch.__file__)), "--expect-target-sha256", PINS["expected_target_sha256"],
                   "--expect-rights-sha256", PINS["expected_rights_sha256"]]
        built = subprocess.run(command, capture_output=True, check=True)
        self.assertEqual(built.stdout, self.raw)
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory, "switch.json")
            file.write_bytes(self.raw)
            result = subprocess.run([*command, "--inspect-contract", str(file), "--expect-contract-sha256", self.pin],
                                    capture_output=True, check=True)
            self.assertEqual(json.loads(result.stdout)["status"], "VALID_PREPARED_SWITCH_DISABLED")
            self.assertEqual(file.read_bytes(), self.raw)
            self.assertEqual(list(Path(directory).iterdir()), [file])
            self.assertEqual(subprocess.run([*command, "--inspect-contract", str(file)], capture_output=True).returncode, 2)


if __name__ == "__main__":
    unittest.main()
