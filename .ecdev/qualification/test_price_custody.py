#!/usr/bin/env python3
"""Refusal tests for the exact source-only selected custody witness."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import price_candidate as candidate
import price_cohort as cohort
import price_custody as custody


class SourceCustody(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pins = {"expected_report_sha256": custody.REPORT_SHA256,
                    "expected_readiness_sha256": custody.READINESS_SHA256}
        cls.value = custody.build_witness(**cls.pins)
        cls.raw = cohort.encode(cls.value)
        cls.pin = cohort.digest(cls.raw)

    def inspect(self, raw=None, pin=None, **kwargs):
        return custody.inspect_witness(self.raw if raw is None else raw,
                                       self.pin if pin is None else pin, **(self.pins | kwargs))

    def reject_changed(self, mutate):
        changed = copy.deepcopy(self.value)
        mutate(changed)
        raw = cohort.encode(changed)
        # A dishonest producer can supply the new digest. Full regeneration must still refuse.
        with self.assertRaises(ValueError):
            self.inspect(raw, cohort.digest(raw))

    def test_roundtrip_binds_four_original_ids_and_every_incident_edge(self):
        checked = self.inspect()
        self.assertEqual(checked["status"], "VALID_SOURCE_WITNESS; CUTOVER_BLOCKED")
        self.assertEqual([row["capture_index"] for row in self.value["selected_nodes"]], [306, 503, 513, 723])
        self.assertEqual({row["role"]: row["original_node"]["id"] for row in self.value["selected_nodes"]},
                         {role: value[1] for role, value in candidate.SOURCE_KEYS.items()})
        self.assertEqual(len(self.value["incident_edges"]), 72)
        self.assertEqual(len(self.value["adjacent_context_nodes"]), 69)
        ids = {row["original_node"]["id"] for row in self.value["selected_nodes"]}
        for row in self.value["incident_edges"]:
            edge = row["original_edge"]
            self.assertTrue(edge["from"] in ids or edge["to"] in ids)
        for row in self.value["selected_nodes"]:
            self.assertEqual(row["original_mapping"]["source_metadata"], row["original_node"])
            self.assertIsNone(row["original_mapping"]["proposed_target"])

    def test_material_bytes_have_both_exact_git_identities_and_original_payloads(self):
        paths = {row["path"] for row in self.value["raw_material"]}
        self.assertTrue(set(custody.MATERIAL_PATHS) <= paths)
        self.assertEqual(len(paths), 71)
        capture = candidate.strict_json(candidate.regular(custody.ROOT / custody.INPUTS["capture"][0]))
        original = {row["path"]: row for row in capture["files"]}
        for row in self.value["raw_material"]:
            self.assertEqual(row["path"], row["historical"]["path"])
            self.assertEqual(row["path"], row["fixed_current"]["path"])
            for version in ("historical", "fixed_current"):
                member = row[version]
                raw = bytes.fromhex(member["hex"])
                self.assertEqual(member["sha256"], cohort.digest(raw))
                self.assertEqual(member["bytes"], len(raw))
                self.assertEqual(member["blob"], custody.blob_id(raw))
                self.assertIn(member["mode"], ("100644", "100755"))
            if row["capture_record"]:
                self.assertEqual(row["historical"]["hex"], original[row["path"]]["hex"])
            if row["path"].startswith(".ecdev/evidence/"):
                self.assertTrue(bytes.fromhex(row["historical"]["hex"]).startswith(b"ECG\x01"))
                self.assertEqual(row["proof_use"],
                                 "HISTORICAL_RECORD_BYTES_ONLY; NOT_A_CURRENT_TEST_OR_TARGET_ATTESTATION")

    def test_historical_caller_proofs_cannot_be_rebound_to_current_source(self):
        row = next(row for row in self.value["raw_material"] if row["path"] == "adapter/web/src/lib.rs")
        self.assertEqual(row["historical"]["sha256"], custody.HISTORICAL_CALLER_SHA256)
        self.assertEqual(row["fixed_current"]["sha256"], custody.caller.CALLER_SHA256)
        self.assertNotEqual(row["historical"]["blob"], row["fixed_current"]["blob"])
        self.assertTrue(row["changed_since_capture"])
        self.assertEqual(self.value["historical_source"]["commit"], custody.HISTORICAL_COMMIT)
        self.assertEqual(self.value["historical_source"]["tree"], custody.HISTORICAL_TREE)
        self.assertEqual(self.value["fixed_current_source"]["commit"], custody.CURRENT_COMMIT)
        self.assertEqual(self.value["fixed_current_source"]["tree"], custody.CURRENT_TREE)
        self.assertFalse(self.value["caller_version_separation"]["historical_proof_reused_as_current_attestation"])
        self.assertEqual(self.value["caller_version_separation"]["current_source_evidence_sha256"], custody.REPORT_SHA256)
        self.reject_changed(lambda value: value["caller_version_separation"].update(
            fixed_current_sha256=custody.HISTORICAL_CALLER_SHA256))
        self.reject_changed(lambda value: value["historical_source"].update(commit=custody.CURRENT_COMMIT))
        self.reject_changed(lambda value: value["fixed_current_source"].update(tree=custody.HISTORICAL_TREE))

    def test_closed_material_list_keeps_full_corpus_as_external_exact_reference(self):
        boundary = self.value["material_boundary"]
        self.assertEqual(boundary["classification"],
                         "PARTIAL_SELECTED_SOURCE_MATERIAL; NOT_COMPLETE_CRATE_OR_RUNTIME_CLOSURE")
        self.assertEqual(len(boundary["historical_proof_record_paths"]), 62)
        self.assertEqual(len(boundary["historical_proof_locator_records"]), 10)
        self.assertTrue(all(boundary["historical_proof_locator_records"].values()))
        rows = self.value["external_material_references"]
        corpus = next(row for row in rows if row["path"] == custody.manifest.CORPUS_PATH)
        self.assertFalse(corpus["bytes_embedded"])
        for version in ("historical", "fixed_current"):
            self.assertEqual(corpus[version]["sha256"], custody.manifest.CORPUS_SHA256)
            self.assertNotIn("hex", corpus[version])
        self.assertTrue(boundary["unclosed_dependencies"])
        self.assertFalse(self.value["preservation"]["source_proposal_rewritten"])

    def test_contract_contains_actual_vectors_exclusions_and_source_boundaries(self):
        contract = self.value["parity_cutover_prerequisites"]
        report = candidate.strict_json(candidate.regular(custody.ROOT / custody.REPORT_PATH))
        readiness = candidate.strict_json(candidate.regular(custody.ROOT / custody.READINESS_PATH))
        self.assertEqual(contract["counts"], report["counts"])
        self.assertEqual(contract["counts"]["web_qualified"], 99)
        self.assertEqual(contract["counts"]["web_old_qualified"], 94)
        self.assertEqual(contract["counts"]["web_new_qualified"], 5)
        self.assertEqual([contract["counts"][key] for key in ("api_qualified", "api_unmapped", "corpus_total")],
                         [106, 2831, 2937])
        self.assertEqual(contract["API_only_exclusions"], report["excluded_cases"])
        self.assertEqual(len(contract["API_only_exclusions"]), 7)
        self.assertEqual(contract["source_boundary_records"], report["negative_cases"])
        self.assertEqual(len(contract["source_boundary_records"]), 20)
        for vector, actual, proposed in zip(contract["vectors"], report["cases"], readiness["proposed_requests"]):
            self.assertEqual(vector["index"], actual["index"])
            self.assertEqual(vector["source_request"], actual["source_request"])
            self.assertEqual(vector["source_Option_presence"], actual["source_request_presence"])
            self.assertEqual(vector["payload_utf8_hex"], actual["source_request"]["input"].encode().hex())
            self.assertEqual(vector["payload_sha256"], proposed["payload_sha256"])
            self.assertEqual(vector["text_selector_facts"], proposed["text_selector_facts"])
            self.assertEqual(vector["expected_source_four_fields"], actual["interpretation"]["result"])
            self.assertEqual(vector["expected_source_float_bits"], actual["interpretation"]["amount_float_bits"])
            self.assertEqual(vector["source_observed"], actual["source_observed"])
            self.assertEqual(vector["target_execution"], "NOT_RUN")
            self.assertIsNone(vector["target_result"])
            self.assertIsNone(vector["canonical_receipt"])
        self.assertEqual(contract["existing_target_reference"], readiness["target_reference"])

    def test_shifted_cohort_option_presence_or_results_are_refused(self):
        mutations = [
            lambda value: value["parity_cutover_prerequisites"]["vectors"].pop(),
            lambda value: value["parity_cutover_prerequisites"]["selected_web_indices"].__setitem__(0, 409),
            lambda value: value["parity_cutover_prerequisites"]["vectors"][0]["source_request"].update(input="29"),
            lambda value: value["parity_cutover_prerequisites"]["vectors"][0]["source_Option_presence"].update(currency_hint=False),
            lambda value: value["parity_cutover_prerequisites"]["vectors"][0].update(expected_source_float_bits="0000000000000000"),
            lambda value: value["parity_cutover_prerequisites"]["counts"].update(api_qualified=99),
            lambda value: value["parity_cutover_prerequisites"]["API_only_exclusions"].clear(),
            lambda value: value["parity_cutover_prerequisites"]["source_boundary_records"].pop(),
        ]
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                self.reject_changed(mutation)

    def test_self_pinned_blob_hex_id_metadata_edge_proof_and_rights_drift_are_refused(self):
        mutations = [
            lambda value: value["raw_material"][0]["historical"].update(hex="00"),
            lambda value: value["raw_material"][0]["fixed_current"].update(blob="0" * 40),
            lambda value: value["raw_material"][0]["historical"].update(mode="100755"),
            lambda value: value["selected_nodes"][0]["original_node"].update(id="ec1:" + "0" * 32),
            lambda value: value["selected_nodes"][1]["original_node"].update(evidence=[]),
            lambda value: value["selected_nodes"][2]["original_mapping"].update(proposed_target={}),
            lambda value: value["incident_edges"][0]["original_edge"].update(kind="IMPLEMENTS"),
            lambda value: value["adjacent_context_nodes"].pop(),
            lambda value: value["material_boundary"]["historical_proof_record_paths"].pop(),
            lambda value: value["source_lineage_and_rights"].update(proof_reinterpretation_performed=True),
            lambda value: value["source_lineage_and_rights"].update(history_completeness="COMPLETE"),
            lambda value: value["source_lineage_and_rights"]["original_rights_claims"][0].update(license_claim="unrestricted"),
        ]
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                self.reject_changed(mutation)

    def test_authority_custody_enrollment_and_current_rights_self_grants_are_refused(self):
        changes = {"canonical_assignment": {}, "accepted_owner": "self", "authority": {},
                   "authority_grants": ["resource.work"], "current_rights": {},
                   "target_exclusive_custody": {}, "Host_admission": {}, "world_admission": {},
                   "source_adapter_cutover": True, "target_execution_performed": True,
                   "canonical_enrollment": "ACCEPTED"}
        for key, value in changes.items():
            with self.subTest(key=key):
                self.reject_changed(lambda witness, key=key, value=value: witness["status"].__setitem__(key, value))
        self.reject_changed(lambda value: value["parity_cutover_prerequisites"].update(
            positive_target_qualification_supported=True))

    def test_source_readiness_refuses_missing_target_execution_custody_authority_and_unknown_effect_evidence(self):
        checked = custody.cutover_readiness(self.raw, self.pin, **self.pins)
        self.assertEqual(checked["status"], "CUTOVER_BLOCKED")
        self.assertEqual(set(checked["missing_target_evidence"]),
                         {"target_execution", "exclusive_custody", "authority", "unknown_effect",
                          "rights_and_enrollment", "production_cutover"})
        for obligation in checked["target_owned_obligations"]:
            self.assertIsNone(obligation["evidence"])
            self.assertTrue(obligation["requires"])
        self.assertEqual(checked["target_execution"], "NOT_RUN")
        for key in ("target_exclusive_custody", "Host_admission", "current_rights", "target_authority_validator"):
            self.assertIsNone(checked[key])
        self.assertFalse(checked["positive_target_qualification_supported"])
        self.assertFalse(checked["source_adapter_cutover"])
        # No positive target fixture exists: even an unqualified empty document has no acceptance path.
        with self.assertRaisesRegex(ValueError, "genuine target authority validator"):
            custody.cutover_readiness(self.raw, self.pin, target_evidence=b"{}", **self.pins)

    def test_independent_witness_report_and_readiness_pins_are_mandatory(self):
        with self.assertRaises(ValueError):
            self.inspect(pin="0" * 64)
        with self.assertRaises(ValueError):
            self.inspect(expected_report_sha256="0" * 64)
        with self.assertRaises(ValueError):
            self.inspect(expected_readiness_sha256="0" * 64)
        with self.assertRaises(TypeError):
            custody.build_witness()
        with self.assertRaises(ValueError):
            self.inspect(raw=b'{"format":"x","format":"y"}', pin=cohort.digest(b'{"format":"x","format":"y"}'))

    def test_immutable_tree_and_current_material_drift_are_refused_without_editing_source(self):
        original_git = custody.immutable.git

        def wrong_tree(root, *args, **kwargs):
            if args == ("rev-parse", custody.CURRENT_COMMIT + "^{tree}"):
                return ("0" * 40 + "\n").encode()
            return original_git(root, *args, **kwargs)

        with mock.patch.object(custody.immutable, "git", side_effect=wrong_tree), self.assertRaises(ValueError):
            custody.build_witness(**self.pins)
        original_read = custody.candidate.regular

        def changed_caller(path):
            raw = original_read(path)
            return raw + b"\n" if Path(path) == custody.ROOT / custody.caller.CALLER_PATH else raw

        with mock.patch.object(custody.candidate, "regular", side_effect=changed_caller), self.assertRaises(ValueError):
            custody.build_witness(**self.pins)

    def test_bounded_reader_refuses_symlink_and_oversize_regular_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            ordinary = root / "ordinary"
            ordinary.write_bytes(b"{}")
            linked = root / "linked"
            linked.symlink_to(ordinary)
            with self.assertRaises(ValueError):
                custody.candidate.regular(linked)
            oversized = root / "oversized"
            with oversized.open("wb") as stream:
                stream.truncate(candidate.MAX_INPUT + 1)
            with self.assertRaises(ValueError):
                custody.candidate.regular(oversized)

    def test_cli_inspects_and_reports_blocked_without_modifying_inputs(self):
        command = [sys.executable, "-B", str(Path(custody.__file__))]
        flags = ["--expect-report-sha256", custody.REPORT_SHA256,
                 "--expect-readiness-sha256", custody.READINESS_SHA256]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "witness.json")
            path.write_bytes(self.raw)
            args = [*command, *flags, "--inspect-witness", str(path), "--expect-witness-sha256", self.pin]
            inspected = subprocess.run(args, check=True, capture_output=True)
            self.assertEqual(json.loads(inspected.stdout)["selected_nodes"], 4)
            blocked = subprocess.run([*args, "--check-cutover"], check=True, capture_output=True)
            self.assertEqual(json.loads(blocked.stdout)["status"], "CUTOVER_BLOCKED")
            contract = subprocess.run([*command, *flags, "--contract"], check=True, capture_output=True)
            self.assertEqual(json.loads(contract.stdout)["target_execution"], "NOT_RUN")
            for bad in ([*args, "--contract"], [*command, *flags, "--check-cutover"],
                        [*command, *flags, "--inspect-witness", str(path)]):
                refused = subprocess.run(bad, capture_output=True)
                self.assertEqual(refused.returncode, 2)
                self.assertFalse(refused.stdout)
            self.assertEqual(path.read_bytes(), self.raw)
            self.assertEqual(list(Path(directory).iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
