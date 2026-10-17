"""The existing custody reader consumes original evidence without Git or adoption."""

import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import price_custody as custody
import price_source_package as package


PINS = {"expected_source_commit": "c973d76bda8476142dd64116b60877942ccb24d5",
        "expected_source_tree": "1020e7ebdc85befdd0e6ee4d73e4417779228f2c",
        "expected_rights_sha256": "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410"}


class CustodyPackageConsumer(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.value = package.build_package(custody.ROOT, **PINS)
        cls.raw = package.encode(cls.value)
        cls.pin = hashlib.sha256(cls.raw).hexdigest()
        cls.evidence = {row["path"]: bytes.fromhex(row["content_hex"])
                        for row in cls.value["evidence"]}
        cls.witness = json.loads(cls.evidence[package.WITNESS_PATH])
        cls.contract = json.loads(cls.evidence[package.PREREQUISITE_PATH])
        cls.source_hashes = cls.material_hashes()

    @classmethod
    def material_hashes(cls):
        return {row["path"]: hashlib.sha256((custody.ROOT / row["path"]).read_bytes()).hexdigest()
                for row in [*cls.value["materials"], *cls.value["evidence"]]}

    def inspect(self, raw=None, pin=None, **pins):
        return custody.inspect_source_package(self.raw if raw is None else raw,
                                              self.pin if pin is None else pin, **(PINS | pins))

    def cli(self, path, *, pin=None):
        return [sys.executable, "-B", str(Path(custody.__file__).absolute()),
                "--inspect-source-package", str(path), "--expect-package-sha256", pin or self.pin,
                "--expect-source-commit", PINS["expected_source_commit"],
                "--expect-source-tree", PINS["expected_source_tree"],
                "--expect-rights-sha256", PINS["expected_rights_sha256"]]

    def test_consumes_exact_original_contract_and_keeps_every_obligation_open(self):
        result = self.inspect()
        self.assertEqual(result["status"], "CUTOVER_BLOCKED")
        self.assertEqual(result["parity_cutover_prerequisites"], self.contract)
        self.assertEqual(result["target_owned_obligations"], self.contract["obligations"])
        self.assertEqual(result["missing_target_evidence"],
                         ["target_execution", "exclusive_custody", "authority", "unknown_effect",
                          "rights_and_enrollment", "production_cutover"])
        for obligation in result["target_owned_obligations"]:
            self.assertIsNone(obligation["evidence"])
            self.assertTrue(obligation["requires"])
        self.assertEqual(result["parity_cutover_prerequisites"]["counts"]["web_qualified"], 99)
        self.assertEqual(len(result["parity_cutover_prerequisites"]["vectors"]), 99)
        self.assertEqual(result["source_status"], self.witness["status"])
        for key in ("current_rights", "target_exclusive_custody", "Host_admission", "target_authority_validator"):
            self.assertIsNone(result[key])
        for key in ("source_adapter_cutover", "positive_target_qualification_supported",
                    "source_checkout_consulted", "source_witness_regenerated"):
            self.assertIs(result[key], False)
        self.assertEqual(result["target_execution"], "NOT_RUN")
        self.assertEqual(result["canonical_enrollment"], "BLOCKED")
        for key in ("context_nodes_adopted", "all828_admitted_mappings", "production_obligations_closed"):
            self.assertEqual(result[key], 0)

    def test_membership_and_original_source_versions_are_kept_distinct(self):
        result = self.inspect()
        self.assertEqual(result["source_package_inspection"]["source_commit"], PINS["expected_source_commit"])
        self.assertEqual(result["source_package_inspection"]["material_files"], 83)
        self.assertEqual(result["historical_source"], self.witness["historical_source"])
        self.assertEqual(result["fixed_current_source"], self.witness["fixed_current_source"])
        self.assertNotEqual(result["fixed_current_source"]["commit"], PINS["expected_source_commit"])
        self.assertEqual(result["caller_version_separation"], self.witness["caller_version_separation"])
        self.assertIs(result["caller_version_separation"]["historical_proof_reused_as_current_attestation"], False)
        self.assertEqual((result["selected_nodes"], result["incident_edges"], result["context_nodes"]), (4, 72, 69))
        self.assertEqual(result["selected_source_bindings"],
                         {row["role"]: row["binding"] for row in self.witness["selected_nodes"]})
        for path, raw in self.evidence.items():
            self.assertEqual(result["original_evidence"][path],
                             {"sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)})

    def test_api_needs_no_checkout_files_git_or_old_witness_regeneration(self):
        with mock.patch.object(custody, "build_witness", side_effect=AssertionError("legacy rebuild")), \
                mock.patch.object(custody.immutable, "git", side_effect=AssertionError("Git read")), \
                mock.patch.object(custody.candidate, "regular", side_effect=AssertionError("source read")), \
                mock.patch.object(custody.candidate, "physical", side_effect=AssertionError("source root")), \
                mock.patch.object(package, "git_object", side_effect=AssertionError("object read")), \
                mock.patch.object(package, "inspect_package_evidence", wraps=package.inspect_package_evidence) as verifier:
            result = self.inspect()
        self.assertEqual(result["status"], "CUTOVER_BLOCKED")
        verifier.assert_called_once()
        with self.assertRaises(TypeError):
            self.inspect(target_evidence=b"{}")

    def test_existing_cli_runs_in_disjoint_directory_without_git_or_source_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "selected-package.json"
            path.write_bytes(self.raw)
            path.chmod(0o444)
            before = path.stat()
            env = dict(os.environ, PATH=str(root / "no-executables"), PYTHONDONTWRITEBYTECODE="1")
            completed = subprocess.run([*self.cli(path), "--source-root", str(root / "absent-checkout")],
                                       cwd=root, env=env, capture_output=True, check=True)
            result = json.loads(completed.stdout)
            self.assertEqual(result["target_owned_obligations"], self.contract["obligations"])
            self.assertEqual(result["original_evidence"][package.WITNESS_PATH]["sha256"],
                             hashlib.sha256(self.evidence[package.WITNESS_PATH]).hexdigest())
            self.assertFalse(result["source_checkout_consulted"])
            self.assertFalse(completed.stderr)
            self.assertEqual(path.read_bytes(), self.raw)
            self.assertEqual((path.stat().st_mtime_ns, path.stat().st_mode),
                             (before.st_mtime_ns, before.st_mode))
            self.assertEqual(list(root.iterdir()), [path])
        self.assertEqual(self.material_hashes(), self.source_hashes)

    def test_all_four_independent_pins_are_required_and_checked(self):
        with self.assertRaises(TypeError):
            custody.inspect_source_package(self.raw, self.pin)
        with self.assertRaises(ValueError):
            self.inspect(pin="0" * 64)
        for name, wrong in [("expected_source_commit", "0" * 40),
                            ("expected_source_tree", "0" * 40),
                            ("expected_rights_sha256", "0" * 64)]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.inspect(**{name: wrong})
        command = self.cli("unused-package")
        for flag in ("--expect-package-sha256", "--expect-source-commit",
                     "--expect-source-tree", "--expect-rights-sha256"):
            args = command.copy()
            index = args.index(flag)
            del args[index:index + 2]
            with self.subTest(flag=flag):
                result = subprocess.run(args, capture_output=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn(b"requires independent", result.stderr)
                self.assertFalse(result.stdout)

    def test_truncated_tampered_and_self_granted_packages_are_refused(self):
        for raw in (self.raw[:-1], self.raw[:len(self.raw) // 2], self.raw + b"\n"):
            with self.subTest(length=len(raw)), self.assertRaises(ValueError):
                self.inspect(raw=raw, pin=hashlib.sha256(raw).hexdigest())
        for key, value in (("current_production_rights", {}), ("canonical_enrollment", "ACCEPTED"),
                           ("target_exclusive_custody", {}), ("source_adapter_cutover", True),
                           ("production_obligations_closed", 6)):
            changed = copy.deepcopy(self.value)
            changed["status"][key] = value
            raw = package.encode(changed)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.inspect(raw=raw, pin=hashlib.sha256(raw).hexdigest())
        changed = copy.deepcopy(self.value)
        changed["materials"][0]["content_hex"] = "00" + changed["materials"][0]["content_hex"][2:]
        raw = package.encode(changed)
        with self.assertRaises(ValueError):
            self.inspect(raw=raw, pin=hashlib.sha256(raw).hexdigest())

    def test_rehashed_original_evidence_cannot_supply_fake_target_evidence(self):
        changed = copy.deepcopy(self.value)
        row = next(row for row in changed["evidence"] if row["path"] == package.PREREQUISITE_PATH)
        contract = copy.deepcopy(self.contract)
        contract["obligations"][0]["evidence"] = {"accepted": True}
        raw = package.encode(contract)
        row.update(content_hex=raw.hex(), bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest(),
                   git_blob=package.git_digest("blob", raw))
        package_raw = package.encode(changed)
        with self.assertRaises(ValueError):
            self.inspect(raw=package_raw, pin=hashlib.sha256(package_raw).hexdigest())

    def test_cli_refuses_mixed_legacy_arguments_and_extraneous_package_pins(self):
        for extra in (["--inspect-witness", "unused"], ["--expect-witness-sha256", "0" * 64],
                      ["--expect-report-sha256", custody.REPORT_SHA256],
                      ["--expect-readiness-sha256", custody.READINESS_SHA256],
                      ["--contract"], ["--check-cutover"]):
            with self.subTest(extra=extra):
                result = subprocess.run([*self.cli("unused-package"), *extra], capture_output=True)
                self.assertEqual(result.returncode, 2)
                self.assertFalse(result.stdout)
        legacy = [sys.executable, "-B", custody.__file__, "--expect-report-sha256", custody.REPORT_SHA256,
                  "--expect-readiness-sha256", custody.READINESS_SHA256]
        for flag in ("--expect-package-sha256", "--expect-source-commit",
                     "--expect-source-tree", "--expect-rights-sha256"):
            with self.subTest(flag=flag):
                result = subprocess.run([*legacy, flag, "0" * 64], capture_output=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn(b"package pins require", result.stderr)
                self.assertFalse(result.stdout)

    def test_legacy_modes_still_require_both_report_and_readiness_pins(self):
        for mode in ([], ["--contract"], ["--inspect-witness", "unused", "--expect-witness-sha256", "0" * 64],
                     ["--check-cutover", "--inspect-witness", "unused", "--expect-witness-sha256", "0" * 64]):
            for flags in ([], ["--expect-report-sha256", custody.REPORT_SHA256],
                          ["--expect-readiness-sha256", custody.READINESS_SHA256]):
                with self.subTest(mode=mode, flags=flags):
                    result = subprocess.run([sys.executable, "-B", custody.__file__, *mode, *flags],
                                            capture_output=True)
                    self.assertEqual(result.returncode, 2)
                    self.assertIn(b"--expect-report-sha256 and --expect-readiness-sha256 are required", result.stderr)
                    self.assertFalse(result.stdout)


if __name__ == "__main__":
    unittest.main()
