"""Complete source routing must preserve identities and never grant ownership."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import source_responsibility as responsibility


class SourceResponsibilities(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.context = responsibility.context(responsibility.ROOT)
        cls.value = responsibility.build()
        cls.raw = responsibility.cohort.encode(cls.value)
        cls.pin = responsibility.cohort.digest(cls.raw)

    def check_changed(self, value):
        raw = responsibility.cohort.encode(value)
        return responsibility.inspect(raw, responsibility.cohort.digest(raw))

    def test_complete_original_identity_metadata_edges_and_payloads_are_retained(self):
        _, capture, proposal, analysis, _ = self.context
        rows = self.value["nodes"]
        self.assertEqual(len(rows), 828)
        self.assertEqual([r["original_pending_mapping"] for r in rows], proposal["node_mappings"])
        self.assertEqual([r["source_evidence"]["metadata"] for r in rows], capture["source_graph"]["nodes"])
        self.assertEqual(self.value["original_edges"], analysis["edges"])
        self.assertEqual(len(self.value["captured_payload_references"]), 734)
        self.assertEqual(self.value["history_and_rights"]["original_rights_claims"], proposal["rights_claims"])
        self.assertEqual(self.value["counts"]["source_kinds"]["CAPABILITY"], 470)
        self.assertEqual(self.value["counts"]["source_kinds"]["PROOF"], 230)
        self.assertTrue(all(r["canonical_assignment"] is None and r["target_custody"] is None for r in rows))

    def test_cohorts_are_complete_disjoint_with_shared_and_location_only_limits(self):
        cohorts = self.value["evidence_cohorts"]
        self.assertEqual(len(cohorts), 15)
        self.assertEqual(sorted(i for row in cohorts for i in row["node_indices"]), list(range(828)))
        counts = {row["id"]: row["count"] for row in cohorts}
        self.assertEqual(counts["donor-study-without-declared-implementation-family"], 380)
        self.assertEqual(counts["shared-explicit-source-relations"], 28)
        only_path = [r for r in self.value["nodes"] if r["source_evidence"]["proof_file_path_family_ids"]
                     and not r["source_evidence"]["declared_graph_or_subject_family_ids"]]
        self.assertEqual(len(only_path), 7)
        self.assertTrue(all(r["physical_implementation_owner"] is None for r in only_path))

    def test_published_facets_cover_source_routes_without_assigning_mixed_families(self):
        _, _, _, _, contracts = self.context
        self.assertEqual(self.value["target_publication"], contracts["target"])
        self.assertEqual(len(self.value["facet_catalog"]), 19)
        keys = {row["key"] for row in contracts["owner_nodes"]}
        for facet in self.value["facet_catalog"].values():
            self.assertTrue(set(facet["reference_owner_keys"]) <= keys)
            self.assertFalse(facet["physical_target_selected"])
            self.assertIsNone(facet["canonical_assignment"])
        rows = self.value["nodes"]
        self.assertTrue(all(r["physical_implementation_owner"] is None for r in rows))
        study = next(r for r in rows if r["source_evidence"]["evidence_cohort"] ==
                     "donor-study-without-declared-implementation-family")
        self.assertEqual(study["candidate_facet_codes"], ["repository-census-provenance"])
        network = next(r for r in rows if r["source_evidence"]["metadata"]["semantic_key"] == "substrate.net")
        self.assertNotIn("replicated-node-transport", network["candidate_facet_codes"])

    def test_only_original_four_price_slices_receive_bounded_reference_proposals(self):
        rows = self.value["nodes"]
        selected = [r for r in rows if r["bounded_correspondence_proposal"] is not None]
        self.assertEqual([r["index"] for r in selected], [306, 503, 513, 723])
        for row in selected:
            proposed = row["bounded_correspondence_proposal"]
            self.assertEqual((proposed["bounded_API_qualified"], proposed["bounded_API_unmapped"]), (106, 2831))
            self.assertEqual((proposed["source_Web_qualified"], proposed["source_Web_excluded_API_cases"]), (99, 7))
            self.assertIn("not the whole source node", proposed["source_slice"])
            self.assertIsNone(proposed["admitted_owner"])
            self.assertIsNone(row["original_pending_mapping"]["proposed_target"])

    def test_primary_source_validation_rejects_identity_lineage_payload_and_rights_changes(self):
        _, capture, proposal, analysis, _ = self.context
        membership = responsibility.candidate.strict_json(responsibility.candidate.regular(
            responsibility.ROOT / responsibility.cohort.ARTIFACTS["source_membership"][0]))
        for mutation in ("node", "order", "edge", "incoming", "payload", "locator", "mode", "rights", "cohort"):
            changed = copy.deepcopy(analysis)
            if mutation == "node":
                changed["nodes"][0]["metadata"]["namespace"] = "other"
            elif mutation == "order":
                changed["nodes"].reverse()
            elif mutation == "edge":
                changed["edges"][0]["scope"] = "RUNTIME"
            elif mutation == "incoming":
                changed["nodes"][0]["incoming_edge_indices"] = []
            elif mutation == "payload":
                changed["captured_payload_references"][0]["sha256"] = "0" * 64
            elif mutation == "locator":
                changed["captured_payload_references"][0]["hex_locator"] = "/files/1/hex"
            elif mutation == "mode":
                changed["captured_payload_references"][0]["mode"] = "100755"
            elif mutation == "rights":
                changed["history_and_rights"]["original_rights_claims"].pop()
            else:
                changed["evidence_cohorts"][0]["node_indices"].pop()
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                responsibility.validate_source(changed, capture, proposal, membership)

    def test_rebound_output_cannot_assign_owner_custody_authority_or_expand_price_scope(self):
        for mutation in ("owner", "custody", "assignment", "grant", "cutover", "price", "edge", "facet", "extra"):
            changed = copy.deepcopy(self.value)
            row = changed["nodes"][0]
            if mutation == "owner":
                row["physical_implementation_owner"] = "capability-kernels"
            elif mutation == "custody":
                row["target_custody"] = {}
            elif mutation == "assignment":
                row["original_pending_mapping"]["proposed_target"] = {}
            elif mutation == "grant":
                changed["authority"]["authority_grants"] = ["resource.work"]
            elif mutation == "cutover":
                changed["authority"]["source_adapter_cutover"] = True
            elif mutation == "price":
                row["bounded_correspondence_proposal"] = changed["nodes"][306]["bounded_correspondence_proposal"]
            elif mutation == "edge":
                changed["original_edges"].pop()
            elif mutation == "facet":
                row["candidate_facet_codes"].append("pure-capability-behavior")
            else:
                row["authority_granted"] = True
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.check_changed(changed)

    def test_inspection_pin_is_independent_and_stale_bytes_fail(self):
        self.assertEqual(responsibility.inspect(self.raw, self.pin)["admitted_mappings"], 0)
        for pin in (None, "", "0" * 64, "sha256:" + self.pin):
            with self.subTest(pin=pin), self.assertRaises(ValueError):
                responsibility.inspect(self.raw, pin)
        with self.assertRaises(ValueError):
            responsibility.inspect(self.raw + b" ", self.pin)

    def test_cli_build_and_read_only_inspection_require_paired_pin(self):
        command = [sys.executable, "-B", str(Path(responsibility.__file__))]
        result = subprocess.run(command, capture_output=True, check=True)
        self.assertEqual(result.stdout, self.raw)
        with tempfile.TemporaryDirectory() as directory:
            file = Path(directory, "candidates.json")
            file.write_bytes(self.raw)
            result = subprocess.run([*command, "--inspect", str(file), "--expect-sha256", self.pin],
                                    capture_output=True, check=True)
            self.assertEqual(json.loads(result.stdout)["status"], "VALID_SOURCE_CANDIDATES_ONLY")
            self.assertEqual(file.read_bytes(), self.raw)
            self.assertEqual(list(Path(directory).iterdir()), [file])
            self.assertEqual(subprocess.run([*command, "--inspect", str(file)], capture_output=True).returncode, 2)


if __name__ == "__main__":
    unittest.main()
