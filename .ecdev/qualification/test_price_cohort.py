"""The retained price cohort cannot be cross-paired or promoted into authority."""

import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import price_cohort as cohort


class ExactPriceCohort(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inputs, cls.corpus = cohort.load_inputs(cohort.ROOT)
        cls.pins = {role: pin for role, (_, pin) in cohort.ARTIFACTS.items()}
        cls.value = cohort.build_cohort(cls.inputs, cls.pins, cls.corpus)
        cls.raw = cohort.encode(cls.value)
        cls.pin = cohort.digest(cls.raw)

    def build(self, inputs=None, pins=None, corpus=None):
        return cohort.build_cohort(self.inputs if inputs is None else inputs,
                                   self.pins if pins is None else pins,
                                   self.corpus if corpus is None else corpus)

    def inspect(self, value=None, pin=None):
        raw = self.raw if value is None else cohort.encode(value)
        return cohort.inspect_cohort(self.inputs, self.pins, self.corpus, raw,
                                     cohort.digest(raw) if pin is None else pin)

    def test_exact_snapshot_membership_and_pending_proposal_roundtrip(self):
        self.assertEqual(self.inspect()["status"], "VALID_PENDING_COHORT")
        snapshot = self.value["source_snapshot"]
        self.assertEqual(snapshot["observed_head"], "db92bf27d31409704fccd44defe342a1b2f22823")
        self.assertEqual(snapshot["source_tree"], "ec88cd6c52fadf3c638bed0eb7b790096eab8617")
        self.assertEqual((snapshot["payloads"], snapshot["nodes"], snapshot["edges"]), (734, 828, 1122))
        self.assertEqual(self.value["complete_proposal"]["target_assignments"], 0)
        self.assertEqual(self.value["status"]["source_authority"], "SOURCE_ONLY")
        self.assertEqual(self.value["status"]["canonical_enrollment"], "BLOCKED")
        self.assertIsNone(self.value["status"]["accepted_owner"])
        self.assertEqual(self.build(pins={k: p.upper() for k, p in self.pins.items()}), self.value)

    def test_all_2831_original_unmapped_rows_and_99_plus_seven_are_bound(self):
        sidecar = json.loads(self.inputs["price_sidecar"])
        coverage = self.value["coverage"]
        self.assertEqual((coverage["total"], coverage["supported"], coverage["unmapped"]), (2937, 106, 2831))
        self.assertEqual(coverage["qualified_case_indices"], sidecar["coverage"]["qualified_case_indices"])
        self.assertEqual(coverage["old99"], sidecar["old99"])
        self.assertEqual(coverage["new_candidate_indices"], [117, 212, 409, 855, 1188, 1193, 1849])
        self.assertEqual(coverage["unmapped_cases_canonical_sha256"],
                         cohort.digest(cohort.encode(sidecar["coverage"]["unmapped_cases"])))
        self.assertEqual(self.value["bounded_price_reference"]["proposed_target"],
                         sidecar["node_mappings"][0]["proposed_target"])

    def test_missing_extra_changed_and_independently_wrong_artifact_pins_refuse(self):
        for role in self.inputs:
            missing = dict(self.inputs)
            missing.pop(role)
            changed = {**self.inputs, role: self.inputs[role] + b" "}
            for inputs, pins in [(missing, self.pins), (changed, self.pins),
                                 (changed, {**self.pins, role: cohort.digest(changed[role])}),
                                 (self.inputs, {**self.pins, role: "0" * 64})]:
                with self.subTest(role=role), self.assertRaises(ValueError):
                    self.build(inputs=inputs, pins=pins)
        with self.assertRaises(ValueError):
            self.build(inputs={**self.inputs, "other": b"inert"})

    def test_new_811_capture_and_proposal_cannot_use_the_old_price_sidecar(self):
        newer = dict(self.inputs)
        pins = dict(self.pins)
        for role, name in [("capture", "source-eight-capture-499a3074-v1.json"),
                           ("proposal", "source-eight-mapping-499a3074-v1.json")]:
            newer[role] = (cohort.ROOT / "research/commerce/governance" / name).read_bytes()
            pins[role] = cohort.digest(newer[role])
        with self.assertRaisesRegex(ValueError, "outside the frozen"):
            self.build(inputs=newer, pins=pins)

    def test_edited_unmapped_code_original_fields_and_target_reference_cannot_be_rebound(self):
        sidecar = json.loads(self.inputs["price_sidecar"])
        for mutation in ("unmapped", "source-field", "target-owner", "admission"):
            value = copy.deepcopy(sidecar)
            if mutation == "unmapped":
                value["coverage"]["unmapped_cases"][0]["code"] += 1
            elif mutation == "source-field":
                value["source_caller"]["used_for_observed_price"] = True
            elif mutation == "target-owner":
                value["node_mappings"][0]["proposed_target"]["existing_canonical_owner"]["key"] = "invented"
            else:
                value["admission"] = "ACCEPTED"
            raw = cohort.encode(value)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.build(inputs={**self.inputs, "price_sidecar": raw},
                           pins={**self.pins, "price_sidecar": cohort.digest(raw)})

    def test_authority_identity_and_typed_boolean_mutations_refuse_even_with_new_cohort_pin(self):
        for field, value in [("canonical_enrollment", "ENROLLED"), ("source_registered", True),
                             ("source_lock_installed", 0), ("canonical_assignment", {}),
                             ("accepted_owner", "invented"), ("authority_grants", ["work"]),
                             ("current_rights", {}), ("source_adapter_cutover", True)]:
            changed = copy.deepcopy(self.value)
            changed["status"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "cohort identity"):
                self.inspect(changed)
        changed = copy.deepcopy(self.value)
        changed["source_bindings"]["source_capability"]["source_id"] = "ec1:" + "0" * 32
        with self.assertRaises(ValueError):
            self.inspect(changed)

    def test_caller_cohort_pin_is_required_and_full_document_is_checked(self):
        for pin in (None, "", "0" * 64, "sha256:" + self.pin, "g" * 64):
            with self.subTest(pin=pin), self.assertRaises(ValueError):
                cohort.inspect_cohort(self.inputs, self.pins, self.corpus, self.raw, pin)
        changed = copy.deepcopy(self.value)
        changed["coverage"].pop("unmapped_cases_canonical_sha256")
        with self.assertRaises(ValueError):
            self.inspect(changed)

    def test_cli_requires_every_pin_emits_exact_bytes_and_inspects_without_writes(self):
        command = [sys.executable, "-B", str(Path(cohort.__file__))]
        flags = [item for role, pin in self.pins.items()
                 for item in ("--expect-" + role.replace("_", "-") + "-sha256", pin)]
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
        built = subprocess.run([*command, *flags], check=True, capture_output=True)
        self.assertEqual(built.stdout, self.raw)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "cohort.json")
            path.write_bytes(self.raw)
            result = subprocess.run([*command, *flags, "--inspect-cohort", str(path),
                                     "--expect-cohort-sha256", self.pin], check=True, capture_output=True)
            self.assertEqual(json.loads(result.stdout)["status"], "VALID_PENDING_COHORT")
            self.assertEqual(list(Path(directory).iterdir()), [path])
            self.assertEqual(path.read_bytes(), self.raw)

    def test_handoff_request_populates_only_source_owned_identity_and_membership(self):
        request = cohort.build_source_handoff(self.inputs, self.pins, self.corpus)
        self.assertEqual(request["cohort"]["sha256"], self.pin)
        self.assertEqual(request["source_identity"]["captured_commit"], self.value["source_snapshot"]["observed_head"])
        self.assertEqual(request["source_identity"]["commit_membership"]["uncaptured_inputs"], 2319)
        self.assertEqual(request["registry_request"], {
            "id": "ecdev", "origin": "llgtrn/ECDEV", "domain": None,
            "status": "DOMAIN_AND_CANONICAL_BINDING_REQUIRE_CENTRAL_DECISION"})
        self.assertFalse(request["authority_handoff"]["handoff_effective"])
        self.assertIsNone(request["authority_handoff"]["canonical_source_owner"])
        self.assertIsNone(request["bounded_price_request"]["whole_source_owner"])
        raw = cohort.encode(request)
        inspected = cohort.inspect_source_handoff(self.inputs, self.pins, self.corpus, raw, cohort.digest(raw))
        self.assertEqual(inspected["canonical_enrollment"], "BLOCKED")

    def test_handoff_does_not_accept_invented_domain_owner_lock_or_authority(self):
        request = cohort.build_source_handoff(self.inputs, self.pins, self.corpus)
        changes = [("registry_request", "domain", "invented"),
                   ("authority_handoff", "canonical_source_owner", "invented"),
                   ("authority_handoff", "canonical_source_identity_mapping", {}),
                   ("authority_handoff", "canonical_authority_acceptance", {}),
                   ("authority_handoff", "accepted_shared_protocol_lock", {}),
                   ("authority_handoff", "handoff_effective", True),
                   ("bounded_price_request", "whole_source_owner", "capability-kernels"),
                   ("bounded_price_request", "authority_grants", ["resource.work"])]
        for section, key, value in changes:
            changed = copy.deepcopy(request)
            changed[section][key] = value
            raw = cohort.encode(changed)
            with self.subTest(key=key), self.assertRaises(ValueError):
                cohort.inspect_source_handoff(self.inputs, self.pins, self.corpus, raw, cohort.digest(raw))

    def test_cli_builds_and_inspects_source_request_and_rejects_mixed_operations(self):
        command = [sys.executable, "-B", str(Path(cohort.__file__))]
        flags = [item for role, pin in self.pins.items()
                 for item in ("--expect-" + role.replace("_", "-") + "-sha256", pin)]
        built = subprocess.run([*command, *flags, "--handoff-request"], check=True, capture_output=True)
        expected = cohort.encode(cohort.build_source_handoff(self.inputs, self.pins, self.corpus))
        self.assertEqual(built.stdout, expected)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "request.json")
            path.write_bytes(expected)
            args = [*command, *flags, "--inspect-request", str(path),
                    "--expect-request-sha256", cohort.digest(expected)]
            result = subprocess.run(args, check=True, capture_output=True)
            self.assertFalse(json.loads(result.stdout)["handoff_effective"])
            self.assertEqual(subprocess.run([*args, "--handoff-request"], capture_output=True).returncode, 2)
            self.assertEqual(list(Path(directory).iterdir()), [path])


if __name__ == "__main__":
    unittest.main()
