"""Use only the existing published package and the three recorded data fixtures."""

import base64
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import selected_package_interop as interop


ROOT = Path(__file__).absolute().parents[2]
PROFILE_PATH = ROOT / "research/commerce/governance/central-selected-package-contract-a095975b-v1.json"
PACKET_DIR = ROOT / "research/commerce/governance/central-selected-package-a095975b"
PACKAGE_PATH = ROOT / "research/commerce/governance/price-source-package-c973d76b-v1.json"
PROFILE_SHA256 = "470d0ea604e0e39a8490d70963e0495506ef25b260464534c9df5199319816d2"
PACKAGE_SHA256 = "be31c8f8067af13502194f81d2642e0eddd184f1856f3b96b64066227a9716bd"
PINS = {
    "expected_central_commit": "a095975b937bf4ef03073ced77949a407b750a7e",
    "expected_central_tree": "08b4df18c97ccdc29d0bb9d02c121192c4a09840",
    "expected_source_commit": "c973d76bda8476142dd64116b60877942ccb24d5",
    "expected_source_tree": "1020e7ebdc85befdd0e6ee4d73e4417779228f2c",
    "expected_rights_sha256": "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410",
}


class RecordedSelectedPackage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Never regenerate or export the source package for this qualification.
        cls.source_raw = PACKAGE_PATH.read_bytes()
        cls.profile_raw = PROFILE_PATH.read_bytes()
        cls.profile = interop.package.strict_json(cls.profile_raw)
        cls.artifacts = {role: (PACKET_DIR / row["path"]).read_bytes()
                         for role, row in cls.profile["artifacts"].items()}
        cls.receipt = interop.package.strict_json(interop.decode_log(cls.artifacts["cli_log"]))

    def inspect(self, *, raw=None, package_pin=PACKAGE_SHA256, profile_raw=None,
                profile_pin=PROFILE_SHA256, artifacts=None, **pins):
        return interop.inspect_recorded_package(
            self.source_raw if raw is None else raw, package_pin,
            profile_raw=self.profile_raw if profile_raw is None else profile_raw,
            expected_profile_sha256=profile_pin,
            artifacts=self.artifacts if artifacts is None else artifacts, **(PINS | pins))

    def rebind(self, *, profile=None, changes=None):
        """Re-pin test mutations to exercise inner checks, never production trust."""
        profile = copy.deepcopy(self.profile if profile is None else profile)
        artifacts = self.artifacts | (changes or {})
        for role, raw in artifacts.items():
            profile["artifacts"][role].update(sha256=interop.package.digest(raw), bytes=len(raw),
                                               git_blob=interop.package.git_digest("blob", raw))
        raw = interop.package.encode(profile)
        return {"profile_raw": raw, "profile_pin": interop.package.digest(raw), "artifacts": artifacts}

    def receipt_mutation(self, raw, *, update_expected=False):
        capture = interop.package.strict_json(self.artifacts["cli_capture"])
        capture.update(bytes=len(raw), sha256=interop.package.digest(raw))
        profile = copy.deepcopy(self.profile)
        if update_expected:
            profile["expected_central_result"] = interop.package.strict_json(raw)
        return self.rebind(profile=profile, changes={"cli_log": base64.encodebytes(raw),
                           "cli_capture": interop.package.encode(capture)})

    def cli_args(self):
        return ["--profile", str(PROFILE_PATH), "--packet-dir", str(PACKET_DIR),
                "--package", str(PACKAGE_PATH), "--expect-profile-sha256", PROFILE_SHA256,
                "--expect-package-sha256", PACKAGE_SHA256,
                *[part for key, value in PINS.items()
                  for part in ("--expect-" + key.removeprefix("expected_").replace("_", "-"), value)]]

    def cli(self, args, cwd):
        return subprocess.run([sys.executable, "-B", str(Path(interop.__file__).absolute()), *args],
                              cwd=cwd, env={"PATH": "", "PYTHONDONTWRITEBYTECODE": "1"},
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False, timeout=30)

    def test_actual_published_receipt_and_exact_local_evidence_keep_six_open(self):
        result = self.inspect()
        self.assertEqual(result["mode"], "RECORDED_PUBLISHED_CLI_ONLY")
        self.assertEqual(result["fresh_central_execution"], "NOT_RUN")
        self.assertEqual(result["status"], "SOURCE_ONLY")
        self.assertEqual(result["central_result"], self.receipt)
        self.assertEqual(result["recorded_receipt_sha256"],
                         "d0c65c639e85ec1bf37609249cf8dc374585e1d018106c938dd5fc6ff918c127")
        self.assertEqual(result["central_evidence_byte_equivalence"], "NOT_ESTABLISHED")
        source = result["source_custody"]
        self.assertEqual(source["status"], "CUTOVER_BLOCKED")
        self.assertEqual(source["missing_target_evidence"], ["target_execution", "exclusive_custody", "authority",
                                                           "unknown_effect", "rights_and_enrollment", "production_cutover"])
        self.assertTrue(all(row["evidence"] is None and row["requires"]
                            for row in source["target_owned_obligations"]))
        value = interop.package.strict_json(self.source_raw)
        evidence = {row["path"]: bytes.fromhex(row["content_hex"]) for row in value["evidence"]}
        self.assertEqual({path: interop.package.digest(raw) for path, raw in evidence.items()}, interop.package.EVIDENCE)
        self.assertEqual(result["local_original_evidence"],
                         {path: {"sha256": interop.package.digest(raw), "bytes": len(raw)}
                          for path, raw in evidence.items()})
        self.assertEqual((self.receipt["material_files"], self.receipt["evidence_files"],
                          self.receipt["tree_objects"]), (83, 3, 19))
        for key in ("authority", "source_adapter_cutover", "source_retirement"):
            self.assertIs(result[key], False)
        for key in ("enrollment_credit", "accepted_obligations"):
            self.assertEqual(result[key], 0)
        self.assertEqual(result["production_obligations"], "ALL_SIX_OPEN")

    def test_unchanged_source_consumer_without_git_subprocess_or_witness_rebuild(self):
        with mock.patch.object(interop.custody, "inspect_source_package",
                               wraps=interop.custody.inspect_source_package) as consumer, \
                mock.patch.object(interop.custody, "build_witness", side_effect=AssertionError("witness rebuild")), \
                mock.patch.object(interop.custody.immutable, "git", side_effect=AssertionError("Git")), \
                mock.patch.object(interop.package, "git_object", side_effect=AssertionError("Git object")), \
                mock.patch.object(interop.package, "build_package", side_effect=AssertionError("package rebuild")), \
                mock.patch.object(subprocess, "run", side_effect=AssertionError("subprocess")), \
                mock.patch.object(interop.custody.candidate, "regular", side_effect=AssertionError("checkout")):
            self.assertTrue(self.inspect()["summary_agreement"])
        consumer.assert_called_once()

    def test_independent_pins_wrong_missing_and_malformed_are_refused(self):
        for key in PINS:
            for value in (None, "", "0" * len(PINS[key]), PINS[key].upper()):
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    self.inspect(**{key: value})
        for key in ("package_pin", "profile_pin"):
            for value in (None, "", "0" * 64):
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    self.inspect(**{key: value})
        kwargs = dict(PINS)
        del kwargs["expected_central_tree"]
        with self.assertRaises(TypeError):
            interop.inspect_recorded_package(self.source_raw, PACKAGE_SHA256,
                                             profile_raw=self.profile_raw, expected_profile_sha256=PROFILE_SHA256,
                                             artifacts=self.artifacts, **kwargs)
        for profile_raw in (None, b"", b"x" * (interop.MAX_PROFILE_BYTES + 1)):
            with self.subTest(profile_type=type(profile_raw).__name__), self.assertRaises(ValueError):
                interop.inspect_recorded_package(
                    self.source_raw, PACKAGE_SHA256, profile_raw=profile_raw,
                    expected_profile_sha256=PROFILE_SHA256, artifacts=self.artifacts, **PINS)

    def test_changed_original_package_and_oversized_package_are_refused(self):
        for raw in (self.source_raw + b" ", b"x" * (interop.package.MAX_PACKAGE_BYTES + 1)):
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                self.inspect(raw=raw)

    def test_packet_roles_and_all_three_byte_identities_are_required(self):
        for role in self.artifacts:
            changed = dict(self.artifacts)
            del changed[role]
            with self.subTest(role=role, mutation="missing"), self.assertRaises(ValueError):
                self.inspect(artifacts=changed)
            changed[role] = self.artifacts[role] + b" "
            with self.subTest(role=role, mutation="tamper"), self.assertRaises(ValueError):
                self.inspect(artifacts=changed)
        with self.assertRaises(ValueError):
            self.inspect(artifacts=self.artifacts | {"extra": b""})

    def test_profile_unknown_fields_and_authority_expansion_are_refused_even_repinned(self):
        for key, value in (("extra", True), ("scope", "PRODUCTION"), ("format", "unknown")):
            profile = copy.deepcopy(self.profile)
            profile[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.inspect(**self.rebind(profile=profile))
        for key, value in (("authority", True), ("enrollment_credit", 1), ("accepted_obligations", 1),
                           ("source_adapter_cutover", True), ("Host_admission", "GRANT"),
                           ("current_production_rights", {}), ("unexpected", True)):
            result = self.receipt | {key: value}
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.inspect(**self.receipt_mutation(interop.package.encode(result), update_expected=True))

    def test_receipt_refusal_unknown_fields_counts_pins_and_types_are_refused(self):
        for key, value in (("status", "REFUSED"), ("status", "TARGET_ADMITTED"), ("extra", 1),
                           ("material_files", 82), ("evidence_files", 2), ("tree_objects", 18),
                           ("package_sha256", "0" * 64), ("source_commit", "0" * 40),
                           ("accepted_obligations", False), ("authority", 0), ("source_retirement", True),
                           ("runtime_admission", "GRANTED"), ("source_execution", "EXECUTED")):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                self.inspect(**self.receipt_mutation(interop.package.encode(self.receipt | {key: value})))

    def test_duplicate_nonfinite_and_malformed_json_refused(self):
        for raw in (b'{"status":"REFUSED","status":"SOURCE_ONLY_SELECTED_MEMBERSHIP"}',
                    b'{"seconds":NaN}', b'{"seconds":1e999}', b'[]', b'{'):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                self.inspect(**self.receipt_mutation(raw))
        for raw in (b'{"format":1,"format":2}', b'{"x":NaN}', b'[]'):
            with self.subTest(profile=raw), self.assertRaises(ValueError):
                self.inspect(profile_raw=raw, profile_pin=interop.package.digest(raw))
        for role in ("cli_capture", "interoperability"):
            for raw in (b'{"x":1,"x":2}', b'{"x":NaN}', b'[]'):
                with self.subTest(role=role, raw=raw), self.assertRaises(ValueError):
                    self.inspect(**self.rebind(changes={role: raw}))

    def test_capture_exit_digest_length_encoding_and_command_tampering(self):
        capture = interop.package.strict_json(self.artifacts["cli_capture"])
        mutations = [("exit", 2), ("exit", False), ("sha256", "0" * 64), ("bytes", 875),
                     ("encoding", "hex"), ("seconds", -1), ("log", "../other"), ("extra", 1)]
        for key, value in mutations:
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.inspect(**self.rebind(changes={"cli_capture": interop.package.encode(capture | {key: value})}))
        changed = copy.deepcopy(capture)
        changed["command"][0] = "/different/binary"
        with self.assertRaises(ValueError):
            self.inspect(**self.rebind(changes={"cli_capture": interop.package.encode(changed)}))

    def test_duplicate_or_missing_recorded_source_flags_refused_even_with_rebound_profile(self):
        for flag in ("--sha256", "--commit", "--tree", "--rights-sha256", "--json", "--root", "--file"):
            profile = copy.deepcopy(self.profile)
            capture = interop.package.strict_json(self.artifacts["cli_capture"])
            index = capture["command"].index(flag)
            capture["command"][index] = "--unknown"
            profile["expected_argv"] = capture["command"]
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                self.inspect(**self.rebind(profile=profile, changes={"cli_capture": interop.package.encode(capture)}))
        profile = copy.deepcopy(self.profile)
        profile["expected_argv"].extend(["--sha256", PACKAGE_SHA256])
        with self.assertRaises(ValueError):
            self.inspect(**self.rebind(profile=profile))

    def test_malformed_noncanonical_and_oversized_base64_refused(self):
        for raw in (b"%%%%", self.artifacts["cli_log"].replace(b"\n", b"\r\n"), b"e30= ",
                    b"e30=\n\n", b"e31=", b"A" * (24 * 1024 + 1)):
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                self.inspect(**self.rebind(changes={"cli_log": raw}))

    def test_interoperability_closed_contract_and_binding_must_match(self):
        original = interop.package.strict_json(self.artifacts["interoperability"])
        for key, value in (("extra", True), ("binding", {}), ("scope", "ADMITTED")):
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.inspect(**self.rebind(changes={"interoperability": interop.package.encode(original | {key: value})}))
        changed = copy.deepcopy(original)
        changed["result_contract"]["authority"] = True
        with self.assertRaises(ValueError):
            self.inspect(**self.rebind(changes={"interoperability": interop.package.encode(changed)}))

    def test_profile_artifact_digest_blob_size_and_unsafe_paths_are_refused(self):
        for key, value in (("sha256", "0" * 64), ("git_blob", "0" * 40), ("bytes", 1),
                           ("path", "../escape"), ("path", "/absolute"), ("published_path", "../escape")):
            profile = copy.deepcopy(self.profile)
            profile["artifacts"]["cli_log"][key] = value
            raw = interop.package.encode(profile)
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                self.inspect(profile_raw=raw, profile_pin=interop.package.digest(raw))

    def test_cli_works_outside_checkout_without_git_and_keeps_fresh_run_not_run(self):
        with tempfile.TemporaryDirectory() as temp:
            result = self.cli(self.cli_args(), temp)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(json.loads(result.stdout)["fresh_central_execution"], "NOT_RUN")

    def test_cli_missing_duplicate_and_wrong_independent_pins_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            for flag in ("--expect-profile-sha256", "--expect-central-commit", "--expect-central-tree",
                         "--expect-package-sha256", "--expect-source-commit", "--expect-source-tree",
                         "--expect-rights-sha256"):
                args = self.cli_args()
                index = args.index(flag)
                value = args[index + 1]
                for changed in (args[:index] + args[index + 2:], args + [flag, value],
                                args[:index + 1] + ["0" * len(value)] + args[index + 2:]):
                    with self.subTest(flag=flag, args=changed[-2:]):
                        result = self.cli(changed, temp)
                        self.assertEqual(result.returncode, 2)
                        self.assertEqual(result.stdout, b"")

    def test_cli_regular_bounded_reads_refuse_symlink_fifo_and_oversize(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            symlink = root / "linked"
            symlink.symlink_to(PROFILE_PATH)
            fifo = root / "fifo"
            os.mkfifo(fifo)
            oversized = root / "large"
            oversized.write_bytes(b"x" * (interop.MAX_PROFILE_BYTES + 1))
            for path in (symlink, fifo, oversized):
                args = self.cli_args()
                args[args.index("--profile") + 1] = str(path)
                with self.subTest(path=path.name):
                    result = self.cli(args, temp)
                    self.assertEqual(result.returncode, 2)
                    self.assertEqual(result.stdout, b"")
            parent_link = root / "parent"
            parent_link.symlink_to(PACKET_DIR, target_is_directory=True)
            args = self.cli_args()
            args[args.index("--packet-dir") + 1] = str(parent_link)
            self.assertEqual(self.cli(args, temp).returncode, 2)


if __name__ == "__main__":
    unittest.main()
