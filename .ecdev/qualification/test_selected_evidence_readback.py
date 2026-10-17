"""Supplied-response comparison tests; these do not prove central execution."""

import builtins
import json
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock

import selected_evidence_readback as readback


ROOT = Path(__file__).absolute().parents[2]
PACKAGE_PATH = ROOT / "research/commerce/governance/price-source-package-c973d76b-v1.json"
PACKAGE_SHA256 = "be31c8f8067af13502194f81d2642e0eddd184f1856f3b96b64066227a9716bd"
PINS = {
    "expected_source_commit": "c973d76bda8476142dd64116b60877942ccb24d5",
    "expected_source_tree": "1020e7ebdc85befdd0e6ee4d73e4417779228f2c",
    "expected_rights_sha256": "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410",
}


class SelectedEvidenceReadback(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Read the existing published artifact; never recreate a package.
        cls.package_raw = PACKAGE_PATH.read_bytes()
        value = readback.package.strict_json(cls.package_raw)
        cls.original = {row["path"]: bytes.fromhex(row["content_hex"]) for row in value["evidence"]}
        cls.responses = {path: {"exit": 0, "stdout": raw, "stderr": b""}
                         for path, raw in cls.original.items()}

    def inspect(self, *, raw=None, package_pin=PACKAGE_SHA256, responses=None, **pins):
        return readback.inspect_readback(
            self.package_raw if raw is None else raw, package_pin,
            responses=self.responses if responses is None else responses, **(PINS | pins))

    def changed(self, path, **fields):
        return self.responses | {path: self.responses[path] | fields}

    def test_three_exact_original_payloads_preserve_all_six_open(self):
        result = self.inspect()
        self.assertEqual(result["status"], "SOURCE_ONLY")
        self.assertEqual(result["scope"], "THREE_ORIGINAL_PINNED_EVIDENCE_PAYLOADS_ONLY")
        self.assertEqual(result["byte_equivalent_payloads"], 3)
        self.assertEqual(result["process_identity"], "NOT_AUTHENTICATED_BY_RESPONSES")
        self.assertEqual(result["evidence"], {
            "research/commerce/governance/price-selected-source-custody-db92bf27-v1.json": {
                "bytes": 2694156, "sha256": "e6f58d934cc1b55d949dc925f83510f5361432f64a3b5620424da3a01bc8d2b0"},
            "research/commerce/governance/price-selected-source-rights-provenance-v1.json": {
                "bytes": 46314, "sha256": "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410"},
            "research/commerce/governance/price-source-parity-cutover-prerequisites-v1.json": {
                "bytes": 152286, "sha256": "6b2a1a93dfe79a503999650826b0d93c6442b022f77380a6738216a515974560"},
        })
        self.assertEqual(result["missing_target_evidence"], ["target_execution", "exclusive_custody", "authority",
                                                           "unknown_effect", "rights_and_enrollment", "production_cutover"])
        self.assertEqual(result["production_obligations"], "ALL_SIX_OPEN")
        self.assertEqual(result["target_execution"], "NOT_RUN")
        self.assertEqual(result["canonical_enrollment"], "BLOCKED")
        self.assertEqual(result["runtime_admission"], "NO_GRANT")
        for key in ("enrollment_credit", "accepted_obligations", "all828_admitted_mappings"):
            self.assertEqual(result[key], 0)
        for key in ("authority", "source_adapter_cutover", "source_retirement", "whole_capture"):
            self.assertIs(result[key], False)
        for key in ("target_exclusive_custody", "Host_admission", "current_production_rights"):
            self.assertIsNone(result[key])

    def test_unchanged_custody_and_package_verification_once_without_io_or_rebuild(self):
        with mock.patch.object(readback.custody, "inspect_source_package",
                               wraps=readback.custody.inspect_source_package) as custody, \
                mock.patch.object(readback.package, "inspect_package_evidence",
                                  wraps=readback.package.inspect_package_evidence) as package, \
                mock.patch.object(readback.package, "build_package", side_effect=AssertionError("package recreation")), \
                mock.patch.object(readback.custody, "build_witness", side_effect=AssertionError("witness rebuild")), \
                mock.patch.object(readback.package, "git_object", side_effect=AssertionError("Git object")), \
                mock.patch.object(readback.custody.immutable, "git", side_effect=AssertionError("Git")), \
                mock.patch.object(subprocess, "run", side_effect=AssertionError("subprocess")), \
                mock.patch.object(subprocess, "Popen", side_effect=AssertionError("process")), \
                mock.patch.object(builtins, "open", side_effect=AssertionError("filesystem")), \
                mock.patch.object(os, "open", side_effect=AssertionError("filesystem descriptor")):
            self.assertEqual(self.inspect()["byte_equivalent_payloads"], 3)
        custody.assert_called_once()
        package.assert_called_once()

    def test_each_independent_pin_missing_wrong_or_malformed_refused(self):
        for key, expected in {"package_pin": PACKAGE_SHA256, **PINS}.items():
            for value in (None, "", "0" * len(expected), expected.upper()):
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    self.inspect(**{key: value})
        for key in PINS:
            pins = dict(PINS)
            del pins[key]
            with self.subTest(missing=key), self.assertRaises(TypeError):
                readback.inspect_readback(self.package_raw, PACKAGE_SHA256, responses=self.responses, **pins)
        with self.assertRaises(TypeError):
            readback.inspect_readback(self.package_raw, responses=self.responses, **PINS)

    def test_changed_original_package_rejected_with_original_and_rebound_digest(self):
        changed = self.package_raw + b" "
        for pin in (PACKAGE_SHA256, readback.package.digest(changed)):
            with self.subTest(pin=pin), self.assertRaises(ValueError):
                self.inspect(raw=changed, package_pin=pin)

    def test_exact_response_path_set_required_no_aliases_material_or_traversal(self):
        for path in self.responses:
            missing = dict(self.responses)
            response = missing.pop(path)
            with self.subTest(path=path, mutation="missing"), self.assertRaises(ValueError):
                self.inspect(responses=missing)
            for alias in ("./" + path, "/" + path, "../" + path, path.replace("/", "\\"),
                          "adapter/web/src/price.rs", "", path.upper()):
                with self.subTest(alias=alias), self.assertRaises(ValueError):
                    self.inspect(responses=missing | {alias: response})
        with self.assertRaises(ValueError):
            self.inspect(responses=self.responses | {"extra": next(iter(self.responses.values()))})
        for responses in ([], tuple(self.responses), "responses"):
            with self.subTest(type=type(responses).__name__), self.assertRaises(ValueError):
                self.inspect(responses=responses)
        with self.assertRaises(ValueError):
            readback.inspect_readback(self.package_raw, PACKAGE_SHA256, responses=None, **PINS)

        class PathAlias(str):
            pass

        path = next(iter(self.responses))
        aliases = dict(self.responses)
        response = aliases.pop(path)
        aliases[PathAlias(path)] = response
        with self.assertRaises(ValueError):
            self.inspect(responses=aliases)

    def test_each_response_is_a_closed_typed_dictionary(self):
        for path in self.responses:
            for field in ("exit", "stdout", "stderr"):
                response = dict(self.responses[path])
                del response[field]
                with self.subTest(path=path, missing=field), self.assertRaises(ValueError):
                    self.inspect(responses=self.responses | {path: response})
            for response in (self.responses[path] | {"authority": True}, [], None):
                with self.subTest(path=path, response_type=type(response).__name__), self.assertRaises(ValueError):
                    self.inspect(responses=self.responses | {path: response})

    def test_each_process_must_succeed_with_exact_integer_exit_and_empty_bytes_stderr(self):
        for path in self.responses:
            for value in (1, 2, 3, -1, False, True, 0.0, "0", None):
                with self.subTest(path=path, exit=value), self.assertRaises(ValueError):
                    self.inspect(responses=self.changed(path, exit=value))
            for value in (b"warning\n", b" ", "", None, bytearray()):
                with self.subTest(path=path, stderr=value), self.assertRaises(ValueError):
                    self.inspect(responses=self.changed(path, stderr=value))

    def test_help_wrapper_reencoding_append_truncate_tamper_and_stdout_types_refused(self):
        for path, original in self.original.items():
            altered = bytes([original[0] ^ 1]) + original[1:]
            reencoded = json.dumps(json.loads(original), sort_keys=True, separators=(",", ":")).encode()
            self.assertNotEqual(reencoded, original)
            candidates = (b"Usage: command --help\n", b'{"payload":' + original + b'}', reencoded,
                          original + b"\n", original[:-1], altered, b"", original.decode(),
                          bytearray(original), memoryview(original), None,
                          b"x" * (readback.MAX_STDOUT_BYTES + 1))
            for raw in candidates:
                with self.subTest(path=path, stdout_type=type(raw).__name__,
                                  size=len(raw) if raw is not None else None), self.assertRaises(ValueError):
                    self.inspect(responses=self.changed(path, stdout=raw))

    def test_direct_byte_equality_still_refuses_an_assumed_digest_collision(self):
        path = readback.package.RIGHTS_PATH
        original = self.original[path]
        changed = bytes([original[0] ^ 1]) + original[1:]
        digest = readback.package.digest

        def assumed_collision(raw):
            return readback.EVIDENCE[path]["sha256"] if raw == changed else digest(raw)

        with mock.patch.object(readback.package, "digest", side_effect=assumed_collision):
            with self.assertRaisesRegex(ValueError, "retained original bytes"):
                self.inspect(responses=self.changed(path, stdout=changed))


if __name__ == "__main__":
    unittest.main()
