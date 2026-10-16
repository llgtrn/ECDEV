"""Actual producer input, retained owner membership and pending-only price candidate checks."""

import copy
import hashlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).absolute().parents[2]
MODULE_PATH = ROOT / ".ecdev/qualification/price_candidate.py"


def load_module():
    spec = importlib.util.spec_from_file_location("price_candidate", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def producer_mapping(helper):
    supplied = os.environ.get("ECDEV_PRICE_CURRENT_MAPPING")
    if supplied:
        return helper.regular(supplied)
    binary = os.environ.get("ECDEV_GOV_BINARY")
    command = [binary] if binary else ["cargo", "run", "--offline", "--release", "--manifest-path",
                                       str(ROOT / ".ecdev/Cargo.toml"), "--"]

    def execute(arguments):
        result = subprocess.run([*command, *arguments, "--root", str(ROOT), "--json"],
                                cwd=ROOT, check=True, capture_output=True, timeout=120)
        if len(result.stdout) > helper.MAX_INPUT:
            raise ValueError("producer output exceeds candidate input budget")
        return result.stdout

    raw = execute(["migrate", "export"])
    with tempfile.TemporaryDirectory() as temporary:
        path = Path(temporary, "capture.json")
        path.write_bytes(raw)
        return execute(["migrate", "propose-mapping", "--from", str(path),
                        "--expect-sha256", hashlib.sha256(raw).hexdigest()])


class PendingPriceCandidate(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.helper = load_module()
        cls.mapping = producer_mapping(cls.helper)
        cls.mapping_hash = cls.helper.digest(cls.mapping)
        cls.owner = cls.helper.regular(ROOT / cls.helper.OWNER_PATH)
        cls.owner_value = cls.helper.strict_json(cls.owner)
        cls.mapping_value = cls.helper.strict_json(cls.mapping)
        cls.candidate = cls.helper.build_candidate(cls.mapping, cls.mapping_hash, cls.owner, ROOT)
        cls.candidate_bytes = (cls.helper.canonical(cls.candidate) + "\n").encode()

    def build(self, mapping=None, pin=None, owner=None):
        return self.helper.build_candidate(self.mapping if mapping is None else mapping,
                                           self.mapping_hash if pin is None else pin,
                                           self.owner if owner is None else owner, ROOT)

    def test_actual_current_mapping_binds_the_existing_source_price_identities(self):
        candidate = self.candidate
        self.assertEqual(candidate["source_mapping"]["source_nodes"], 828)
        self.assertEqual(candidate["source_mapping"]["sha256"], self.mapping_hash)
        self.assertEqual(candidate["source_mapping"]["target_assignments"], 0)
        self.assertIsNone(self.mapping_value["target_custody"])
        self.assertTrue(all(row["proposed_target"] is None for row in self.mapping_value["node_mappings"]))
        nodes = self.mapping_value["node_mappings"]
        for role, (key, identity) in self.helper.SOURCE_KEYS.items():
            selected = candidate["source_bindings"][role]
            index = int(selected["mapping_locator"].rsplit("/", 1)[1])
            self.assertEqual((nodes[index]["source_semantic_key"], nodes[index]["source_id"]), (key, identity))
            self.assertEqual(selected["capture_locator"], nodes[index]["capture_locator"])
        self.assertEqual(candidate["source_caller"]["entry"], "adapter/web/src/lib.rs::products")
        self.assertFalse(candidate["source_caller"]["used_for_observed_price"])
        self.assertFalse(candidate["source_caller"]["currency_token_is_iso_identity"])

    def test_all_2937_cases_are_accounted_for_and_the_old99_are_not_rebound(self):
        coverage = self.candidate["coverage"]
        self.assertEqual((coverage["total"], coverage["supported"], coverage["unmapped"]), (2937, 106, 2831))
        supported = coverage["qualified_case_indices"]
        unavailable = coverage["unmapped_cases"]
        self.assertEqual(len(supported), 106)
        self.assertEqual(len(unavailable), 2831)
        self.assertFalse(set(supported) & {row["index"] for row in unavailable})
        self.assertEqual(set(supported) | {row["index"] for row in unavailable}, set(range(2937)))
        self.assertTrue(all(row["target"] is None and type(row["code"]) is int for row in unavailable))
        self.assertEqual([row["index"] for row in self.candidate["new_candidates"]],
                         [117, 212, 409, 855, 1188, 1193, 1849])
        self.assertTrue(all(row["canonical_assignment"] is None for row in self.candidate["new_candidates"]))
        self.assertEqual(self.candidate["old99"]["count"], 99)
        self.assertEqual(self.candidate["old99"]["status"], "UNCHANGED_PRIOR_QUALIFICATION; NOT_REBOUND")
        self.assertEqual(len(coverage["reason_only_changes"]), 27)

    def test_independent_git_membership_proves_the_declared_owner_only(self):
        checked = self.helper.validate_owner(self.owner_value, self.candidate["target_binding"])
        self.assertEqual(checked, self.candidate["declared_owner_verification"])
        self.assertEqual(checked["status"], "VERIFIED_DECLARED_SOURCE_NODE_MEMBERSHIP_ONLY")
        self.assertEqual(checked["declaration_git_blob"], "510f1885637c9730624844c16ffb0c40dabbeaa1")
        self.assertEqual(checked["provided_capability"], "commerce.price.parse-text")
        self.assertEqual(checked["owner"]["key"], "capability-kernels")
        self.assertIsNone(checked["current_owner_acceptance"])
        self.assertEqual(self.candidate["target_binding"]["capability_NodeId"], "cn2:964273aabb7ba94d6c4c0c421a2730d1")
        self.assertEqual(self.candidate["target_binding"]["implementation_NodeId"], "cn2:64c339cb2b639c152c053cc3f88e4cf0")
        self.assertIn("CustodyUnverified", self.candidate["target_binding"]["activation"])

    def test_owner_commit_tree_blob_key_path_and_provided_capability_cannot_change(self):
        for kind in ["commit", "tree", "blob", "body", "duplicate-tree", "tree-member"]:
            with self.subTest(kind=kind):
                value = copy.deepcopy(self.owner_value)
                if kind == "commit":
                    value["commit_preimage_utf8"] += "invented\n"
                elif kind == "tree":
                    value["tree"] = "0" * 40
                elif kind == "blob":
                    value["file"]["git_blob"] = "0" * 40
                elif kind == "body":
                    value["file"]["content_utf8"] = value["file"]["content_utf8"].replace("capability-kernels", "invented-owner")
                elif kind == "duplicate-tree":
                    value["tree_witness"][1] = copy.deepcopy(value["tree_witness"][0])
                else:
                    value["tree_witness"][0]["entries"][0]["oid"] = "0" * 40
                with self.assertRaises((ValueError, KeyError, StopIteration)):
                    self.helper.validate_owner(value, self.candidate["target_binding"])
                with self.assertRaises(ValueError):
                    self.build(owner=(self.helper.canonical(value) + "\n").encode())
        for field, changed in [("key", "invented-owner"), ("path", "invented/path")]:
            target = copy.deepcopy(self.candidate["target_binding"])
            target["owner"][field] = changed
            with self.assertRaises(ValueError):
                self.helper.validate_owner(self.owner_value, target)
        target = copy.deepcopy(self.candidate["target_binding"])
        target["capability"] = "commerce.price.invented"
        with self.assertRaises(ValueError):
            self.helper.validate_owner(self.owner_value, target)

    def test_caller_bound_mapping_pin_is_mandatory_and_case_insensitive(self):
        self.assertEqual(self.build(pin=self.mapping_hash.upper()), self.candidate)
        for pin in ["", "0" * 63, "z" * 64, "0" * 64, "sha256:" + self.mapping_hash]:
            with self.subTest(pin=pin[:16]), self.assertRaises(ValueError):
                self.build(pin=pin)
        with self.assertRaises(ValueError):
            self.build(mapping=self.mapping + b" ")
        with self.assertRaises(ValueError):
            self.build(mapping=b" " * (self.helper.MAX_INPUT + 1))

    def test_source_rekey_coverage_target_and_real_caller_changes_are_refused_even_when_rebound(self):
        for kind in ["rekey", "drop-node", "drop-payload", "grant", "node-grant", "rights", "caller"]:
            with self.subTest(kind=kind):
                value = copy.deepcopy(self.mapping_value)
                if kind == "rekey":
                    value["node_mappings"][0]["source_id"] = "ec1:" + "0" * 32
                elif kind == "drop-node":
                    value["node_mappings"].pop()
                elif kind == "drop-payload":
                    value["payloads"]["evidence"].pop()
                elif kind == "grant":
                    value["target_custody"] = {"accepted": True}
                elif kind == "node-grant":
                    value["node_mappings"][0]["proposed_target"] = "self-assigned"
                elif kind == "rights":
                    value["rights_claims"][0]["license_claim"] = "unrestricted"
                else:
                    index = next(i for i, row in enumerate(value["node_mappings"])
                                 if row["source_semantic_key"] == "commerce.web-research")
                    for node in [value["node_mappings"][index]["source_metadata"],
                                 value["source_semantics"]["source_graph"]["nodes"][index]]:
                        node["capabilities"].remove("extract.price-public-api")
                raw = (self.helper.canonical(value) + "\n").encode()
                with self.assertRaises((ValueError, KeyError)):
                    self.build(mapping=raw, pin=self.helper.digest(raw))

    def test_current_rights_build_assessment_and_admission_stay_unknown_or_pending(self):
        value = self.candidate
        self.assertEqual(value["copying_authorization"], "UNKNOWN")
        self.assertEqual(value["admission"], "PENDING")
        for key in ["canonical_assignment", "accepted_owner", "current_rights", "build_assessment", "binary_attestation", "world_admission"]:
            self.assertIsNone(value[key], key)
        self.assertFalse(value["source_adapter_cutover"])
        self.assertFalse(value["governed_caller_locally_run"])
        self.assertEqual(value["authority_grants"], [])
        self.assertEqual(value["source8_registry"]["status"], "PENDING")
        self.assertFalse(value["source8_registry"]["granted_by_this_candidate"])

    def test_inspection_rejects_self_grants_subset_shrinkage_and_duplicate_json(self):
        check = self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner, self.candidate_bytes, ROOT)
        self.assertEqual(check["status"], "VALID_PENDING_CAPABILITY_CANDIDATE")
        for key in ["canonical_assignment", "accepted_owner", "build_assessment", "admission", "copying_authorization"]:
            value = copy.deepcopy(self.candidate)
            value[key] = "self-approved"
            with self.assertRaises(ValueError):
                self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner,
                                              (self.helper.canonical(value) + "\n").encode(), ROOT)
        value = copy.deepcopy(self.candidate)
        value["coverage"]["unmapped_cases"].pop()
        with self.assertRaises(ValueError):
            self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner,
                                          (self.helper.canonical(value) + "\n").encode(), ROOT)
        for raw in [self.candidate_bytes.replace(b"{", b'{"admission":"duplicate",', 1),
                    self.candidate_bytes.replace(b"{", b'{"\\u0061dmission":"duplicate",', 1)]:
            with self.assertRaises(ValueError):
                self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner, raw, ROOT)

    def test_inspector_preserves_nested_boolean_and_integer_json_types(self):
        for path, replacement in [(("source_adapter_cutover",), 0),
                                  (("source8_registry", "granted_by_this_candidate"), 0),
                                  (("coverage", "supported"), 106.0),
                                  (("coverage", "qualified_case_indices", 0), 0.0)]:
            value = copy.deepcopy(self.candidate)
            parent = value
            for key in path[:-1]:
                parent = parent[key]
            # Retype a case index without changing its numeric value.
            if path[-1] == 0:
                replacement = float(parent[0])
            parent[path[-1]] = replacement
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner,
                                              (self.helper.canonical(value) + "\n").encode(), ROOT)

    def test_original99_and_eur_receipts_and_mapping_are_unchanged(self):
        retained = ROOT / "research/commerce/governance/price-text-subset-proposal-v1.json"
        before = retained.read_bytes()
        self.assertEqual(self.helper.digest(before), "2e05d72ca01d42bc537bbfba8a9c03c398008e495a5abf5af6e032110923e9cf")
        expected = self.helper.reconcile(ROOT)
        self.assertEqual(expected["old99"], self.candidate["old99"])
        self.assertEqual(expected["new_candidates"], self.candidate["new_candidates"])
        self.assertEqual(retained.read_bytes(), before)
        self.assertEqual(self.helper.strict_json(self.mapping), self.mapping_value)
        self.assertEqual(self.build(), self.candidate)

    def test_cli_runs_actual_candidate_and_rejects_existing_or_source_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            mapping = directory / "mapping.json"
            mapping.write_bytes(self.mapping)
            output = directory / "result"
            process = subprocess.run([sys.executable, "-B", str(MODULE_PATH), "--mapping", str(mapping),
                                      "--expect-mapping-sha256", self.mapping_hash, "--output", str(output)],
                                     cwd=ROOT, capture_output=True, timeout=30)
            self.assertEqual(process.returncode, 0, process.stderr.decode())
            self.assertEqual((output / "price-capability-candidate-v1.json").read_bytes(), self.candidate_bytes)
            with self.assertRaises(ValueError):
                self.helper.run(mapping, self.mapping_hash, output, ROOT)
            with self.assertRaises(ValueError):
                self.helper.run(mapping, self.mapping_hash, ROOT / "forbidden-price-candidate-output", ROOT)
            self.assertEqual(mapping.read_bytes(), self.mapping)

    def test_nonregular_inputs_and_changed_retained_helpers_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary, "input")
            path.symlink_to(ROOT / self.helper.OWNER_PATH)
            with self.assertRaises(ValueError):
                self.helper.regular(path)
            with self.assertRaises(ValueError):
                self.helper.regular(Path(temporary))
        with patch.object(self.helper, "EUR_SHA256", "0" * 64), self.assertRaises(ValueError):
            self.build()


class BoundedPriceMapping(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.helper = load_module()
        cls.mapping = producer_mapping(cls.helper)
        cls.mapping_hash = cls.helper.digest(cls.mapping)
        cls.owner = cls.helper.regular(ROOT / cls.helper.OWNER_PATH)
        cls.target = cls.helper.regular(ROOT / cls.helper.TARGET_CONTRACT_PATH)
        cls.target_hash = cls.helper.digest(cls.target)
        cls.contract = cls.helper.strict_json(cls.target)
        cls.value = cls.helper.build_bounded_mapping(cls.mapping, cls.mapping_hash, cls.owner,
                                                    cls.target, cls.target_hash, ROOT)
        cls.raw = (cls.helper.canonical(cls.value) + "\n").encode()

    def build(self, target=None, pin=None):
        return self.helper.build_bounded_mapping(self.mapping, self.mapping_hash, self.owner,
                                                 self.target if target is None else target,
                                                 self.target_hash if pin is None else pin, ROOT)

    def inspect(self, value):
        return self.helper.inspect_bounded_mapping(self.mapping, self.mapping_hash, self.owner,
                                                   self.target, self.target_hash,
                                                   (self.helper.canonical(value) + "\n").encode(), ROOT)

    def test_exactly_one_existing_source_capability_has_the_reviewed_concrete_target(self):
        value = self.value
        self.assertEqual(value["format"], "ecdev-bounded-price-mapping-v1")
        self.assertEqual(value["scope"], self.helper.BOUNDED_SCOPE)
        self.assertEqual(len(value["source_bindings"]), 4)
        self.assertEqual(len(value["node_mappings"]), 1)
        mapped = value["node_mappings"][0]
        self.assertEqual(mapped["source_id"], "ec1:a2e3b3e884070b64d4edacda08adf6e6")
        self.assertEqual(mapped["source_semantic_key"], "capability/extract.price-public-api")
        self.assertEqual(mapped["proposed_target"], self.contract["target"])
        self.assertEqual(mapped["proposed_target"]["commit"], "d7c54f7ec836da6a8459f8a3786964ea7c674866")
        self.assertEqual(mapped["proposed_target"]["tree"], "2965cd433ee1f744775b8cc885dc7cbab3efb6fb")
        self.assertEqual(value["target_contract"]["sha256"], self.target_hash)
        self.assertEqual(value["target_contract"]["git_blob"], "89e449e57d4ad1ba5ee45c9d3c0f98e7af012455")
        self.assertEqual(self.inspect(value)["status"], "VALID_PENDING_BOUNDED_MAPPING")
        self.assertEqual(self.build(pin=self.target_hash.upper()), value)

    def test_legacy_pending_proposals_and_inspector_do_not_accept_the_new_positive_format(self):
        legacy = self.helper.build_candidate(self.mapping, self.mapping_hash, self.owner, ROOT)
        source = self.helper.strict_json(self.mapping)
        self.assertIsNone(source["target_custody"])
        self.assertTrue(all(row["proposed_target"] is None for row in source["node_mappings"]))
        self.assertIsNone(legacy["source_mapping"]["target_custody"])
        self.assertEqual(legacy["source_mapping"]["target_assignments"], 0)
        with self.assertRaises(ValueError):
            self.helper.inspect_candidate(self.mapping, self.mapping_hash, self.owner, self.raw, ROOT)
        with self.assertRaises(ValueError):
            self.inspect(legacy)
        changed = copy.deepcopy(self.value)
        changed["format"] = legacy["format"]
        with self.assertRaises(ValueError):
            self.inspect(changed)

    def test_target_contract_requires_an_independent_pin_and_the_exact_reviewed_version(self):
        for pin in ["", "0" * 63, "z" * 64, "0" * 64, "sha256:" + self.target_hash]:
            with self.subTest(pin=pin[:16]), self.assertRaises(ValueError):
                self.build(pin=pin)
        with self.assertRaises(ValueError):
            self.build(target=self.target + b" ")
        for kind in ["target-pin", "owner", "capability", "implementation", "coverage", "source", "authority"]:
            with self.subTest(kind=kind):
                changed = copy.deepcopy(self.contract)
                if kind == "target-pin":
                    changed["target"]["commit"] = self.helper.TARGET_COMMIT
                elif kind == "owner":
                    changed["target"]["existing_canonical_owner"]["key"] = "invented-owner"
                elif kind == "capability":
                    changed["target"]["capability_NodeId"] = "cn2:" + "0" * 32
                elif kind == "implementation":
                    changed["target"]["implementation_NodeId"] = "cn2:" + "0" * 32
                elif kind == "coverage":
                    changed["bounded_coverage"]["supported"] = 2937
                elif kind == "source":
                    changed["source"]["native_source_capability"] = self.helper.SOURCE_KEYS["donor_capability"][1]
                else:
                    changed["no_grant"]["authority"] = True
                raw = (self.helper.canonical(changed) + "\n").encode()
                with self.assertRaises(ValueError):
                    self.build(target=raw, pin=self.helper.digest(raw))

    def test_other_source_overreach_target_drift_full_scope_and_self_grants_are_refused(self):
        for kind in ["other-source", "extra-source", "target", "scope", "accepted_owner", "authority", "canonical_assignment", "custody"]:
            with self.subTest(kind=kind):
                changed = copy.deepcopy(self.value)
                if kind == "other-source":
                    changed["node_mappings"][0]["source_id"] = self.helper.SOURCE_KEYS["donor_capability"][1]
                elif kind == "extra-source":
                    changed["node_mappings"].append(copy.deepcopy(changed["node_mappings"][0]))
                elif kind == "target":
                    changed["node_mappings"][0]["proposed_target"]["capability_NodeId"] = "cn2:" + "0" * 32
                elif kind == "scope":
                    changed["scope"] = "ENTIRE_SOURCE_CAPABILITY_ADOPTED"
                elif kind == "custody":
                    changed["target_custody"]["exclusive_execution_custody"] = "ESTABLISHED"
                else:
                    changed[kind] = "self-approved"
                with self.assertRaises(ValueError):
                    self.inspect(changed)

    def test_bounded_inspector_preserves_nested_boolean_and_integer_json_types(self):
        for path, replacement in [(("source_adapter_cutover",), 0),
                                  (("source8_registry", "granted_by_this_candidate"), 0),
                                  (("coverage", "supported"), 106.0),
                                  (("coverage", "qualified_case_indices", 0), 0.0)]:
            value = copy.deepcopy(self.value)
            parent = value
            for key in path[:-1]:
                parent = parent[key]
            if path[-1] == 0:
                replacement = float(parent[0])
            parent[path[-1]] = replacement
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.inspect(value)

    def test_all_coverage_and_historical_unknown_limits_survive_the_positive_proposal(self):
        value = self.value
        legacy = self.helper.build_candidate(self.mapping, self.mapping_hash, self.owner, ROOT)
        self.assertEqual(value["coverage"], legacy["coverage"])
        self.assertEqual(value["old99"], legacy["old99"])
        self.assertEqual(value["historical_subset_publication"], legacy["target_publication"])
        self.assertEqual([c["index"] for c in value["new_candidates"]], [117, 212, 409, 855, 1188, 1193, 1849])
        self.assertTrue(all(c["proposed_target_locator"] == "/node_mappings/0/proposed_target" for c in value["new_candidates"]))
        self.assertTrue(all(c["canonical_assignment"] is None for c in value["new_candidates"]))
        self.assertEqual(value["target_custody"]["status"], "PUBLISHED_TARGET_SOURCE_REFERENCE_ONLY")
        self.assertEqual(value["target_custody"]["runtime_custody"], "NOT_ESTABLISHED")
        self.assertEqual(value["target_custody"]["exclusive_execution_custody"], "NOT_ESTABLISHED")
        for key in ["canonical_assignment", "accepted_owner", "authority", "current_rights", "build_assessment", "binary_attestation", "world_admission"]:
            self.assertIsNone(value[key], key)
        self.assertEqual(value["copying_authorization"], "UNKNOWN")
        self.assertEqual(value["admission"], "PENDING")
        self.assertEqual(value["source8_registry"]["status"], "PENDING")
        self.assertFalse(value["source_adapter_cutover"])
        self.assertFalse(value["governed_caller_locally_run"])

    def test_bounded_cli_requires_paired_target_arguments_and_writes_only_the_new_format(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            mapping = directory / "mapping.json"
            mapping.write_bytes(self.mapping)
            output = directory / "bounded"
            command = [sys.executable, "-B", str(MODULE_PATH), "--mapping", str(mapping),
                       "--expect-mapping-sha256", self.mapping_hash, "--output", str(output),
                       "--target-contract", str(ROOT / self.helper.TARGET_CONTRACT_PATH)]
            missing = subprocess.run(command, cwd=ROOT, capture_output=True, timeout=30)
            self.assertEqual(missing.returncode, 2)
            self.assertFalse(output.exists())
            actual = subprocess.run([*command, "--expect-target-contract-sha256", self.target_hash],
                                    cwd=ROOT, capture_output=True, timeout=30)
            self.assertEqual(actual.returncode, 0, actual.stderr.decode())
            self.assertEqual((output / "bounded-price-mapping-v1.json").read_bytes(), self.raw)
            self.assertTrue((output / "bounded-inspection.json").is_file())
            self.assertFalse((output / "price-capability-candidate-v1.json").exists())
            self.assertEqual(mapping.read_bytes(), self.mapping)


if __name__ == "__main__":
    unittest.main()
