"""Local refusal tests for candidate custody; no engine, donor, network or Git clone runs."""

import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("source_candidate_runner", Path(__file__).with_name("run_candidate.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

SYNTHETIC_IDS = [f"fixture-{letter}" for letter in "abcdefg"]


def blob_id(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()


def binding_for(files):
    rows = [{"path": path, "sha256": runner.digest(data), "bytes": len(data), "executable": False}
            for path, data in sorted(files.items())]
    return {"observed_head": "a" * 40, "inputs": rows, "index_paths": [row["path"] for row in rows],
            "manifest_sha256": runner.digest(runner.canonical(rows))}


class Membership(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.files = {path: (path + "\n").encode() for path in runner.REQUIRED_INPUTS}
        self.files["product.rs"] = b"pub fn source() {}\n"
        for path, data in self.files.items():
            file = self.root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(data)
            file.chmod(0o644)
        self.binding = binding_for(self.files)
        self.entries = {path: ("100644", blob_id(data)) for path, data in self.files.items()}
        self.indexed = list(self.binding["index_paths"])
        self.heads = ["a" * 40, "a" * 40]
        self.batch_override = None

    def git(self, _source, *args, input_bytes=None):
        if args == ("rev-parse", "HEAD^{commit}"):
            return (self.heads.pop(0) + "\n").encode()
        if args == ("rev-parse", "a" * 40 + "^{tree}"):
            return ("b" * 40 + "\n").encode()
        if args[0] == "ls-files":
            return b"\0".join(path.encode() for path in self.indexed) + b"\0"
        if args[0] == "ls-tree":
            return b"".join(f"{mode} blob {oid}\t{path}\0".encode()
                            for path, (mode, oid) in sorted(self.entries.items()))
        if args == ("cat-file", "--batch"):
            if self.batch_override is not None:
                return self.batch_override
            objects = {blob_id(data): data for data in self.files.values()}
            return b"".join(oid + b" blob " + str(len(objects[oid.decode()])).encode() + b"\n"
                            + objects[oid.decode()] + b"\n" for oid in input_bytes.splitlines())
        raise AssertionError(args)

    def verify(self):
        with patch.object(runner, "git", side_effect=self.git):
            return runner.git_membership(self.root, self.binding)

    def test_complete_membership_matches_every_blob_mode_and_worktree(self):
        before = {path: (self.root / path).stat().st_mtime_ns for path in self.files}
        report, data = self.verify()
        self.assertEqual(data, self.files)
        self.assertEqual(len(report["inputs"]), len(self.files))
        self.assertEqual(report["source_commit"], self.binding["observed_head"])
        self.assertEqual(report["manifest_sha256"], runner.digest(runner.canonical(report["inputs"])))
        self.assertIsNone(report["rights_authorization"])
        self.assertIn("NOT_MIGRATED", report["runtime_data"])
        self.assertEqual(before, {path: (self.root / path).stat().st_mtime_ns for path in self.files})

    def test_missing_index_and_missing_committed_paths_refuse(self):
        self.indexed.pop()
        with self.assertRaisesRegex(ValueError, "index paths"):
            self.verify()
        self.indexed = self.binding["index_paths"]
        self.entries.pop("product.rs")
        with self.assertRaisesRegex(ValueError, "committed paths"):
            self.verify()

    def test_staged_but_uncommitted_program_is_not_membership(self):
        self.entries.pop(runner.CANDIDATE_SOURCE)
        with self.assertRaisesRegex(ValueError, "committed paths"):
            self.verify()

    def test_initial_and_postread_head_changes_refuse(self):
        self.heads = ["c" * 40]
        with self.assertRaisesRegex(ValueError, "HEAD changed before"):
            self.verify()
        self.heads = ["a" * 40, "c" * 40]
        with self.assertRaisesRegex(ValueError, "HEAD changed during"):
            self.verify()

    def test_unsupported_tree_mode_and_worktree_mode_changes_refuse(self):
        self.entries["product.rs"] = ("120000", blob_id(self.files["product.rs"]))
        with self.assertRaisesRegex(ValueError, "unsupported"):
            self.verify()
        self.heads = ["a" * 40, "a" * 40]
        self.entries["product.rs"] = ("100644", blob_id(self.files["product.rs"]))
        (self.root / "product.rs").chmod(0o755)
        with self.assertRaisesRegex(ValueError, "mode differ"):
            self.verify()

    def test_changed_worktree_and_manifest_digest_refuse(self):
        (self.root / "product.rs").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "bytes or mode differ"):
            self.verify()
        self.binding["manifest_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "manifest digest"):
            self.verify()

    def test_bound_digest_must_match_committed_bytes(self):
        self.binding["inputs"][-1]["sha256"] = "0" * 64
        self.binding["manifest_sha256"] = runner.digest(runner.canonical(self.binding["inputs"]))
        with self.assertRaisesRegex(ValueError, "bytes or mode differ"):
            self.verify()

    def test_missing_blob_and_truncated_blob_refuse(self):
        self.batch_override = b"0" * 40 + b" missing\n"
        with self.assertRaisesRegex(ValueError, "missing or invalid"):
            self.verify()
        self.heads = ["a" * 40, "a" * 40]
        self.batch_override = b"truncated"
        with self.assertRaisesRegex(ValueError, "truncated"):
            self.verify()

    def test_blob_object_identity_is_independently_recomputed(self):
        data = self.files["product.rs"]
        self.entries = {"product.rs": ("100644", "0" * 40)}
        self.batch_override = b"0" * 40 + f" blob {len(data)}\n".encode() + data + b"\n"
        with patch.object(runner, "git", side_effect=self.git):
            with self.assertRaisesRegex(ValueError, "object identity"):
                runner.committed_blobs(self.root, self.entries, {"product.rs": {"bytes": len(data)}})

    def test_unsafe_duplicate_and_unindexed_candidate_inputs_refuse(self):
        for mutation in ("unsafe", "duplicate", "omitted", "boolean"):
            binding = copy.deepcopy(self.binding)
            if mutation == "unsafe":
                binding["inputs"][0]["path"] = "../outside"
            elif mutation == "duplicate":
                binding["inputs"].append(binding["inputs"][0])
            elif mutation == "omitted":
                binding["inputs"] = [row for row in binding["inputs"] if row["path"] != runner.CANDIDATE_SOURCE]
                binding["index_paths"] = [row["path"] for row in binding["inputs"]]
            else:
                binding["inputs"][0]["bytes"] = True
            binding["manifest_sha256"] = runner.digest(runner.canonical(binding["inputs"]))
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                runner.rows_of(binding)


class Capture(unittest.TestCase):
    def setUp(self):
        self.head = "a" * 40
        self.committed = {f".ecdev/evidence/{index:032x}.ecg": str(index).encode() for index in range(734)}
        self.capture = {"format": "ecdev-source-capture-v1",
                        "binding": "CAPTURED_BYTES; HEAD_OBSERVED_ONLY; NOT_ENROLLED; NOT_REVERIFIED",
                        "source_id": "ecdev", "source_repository": "llgtrn/ECDEV", "observed_head": self.head,
                        "files": [{"path": path, "sha256": runner.digest(data), "hex": data.hex()}
                                  for path, data in self.committed.items()]}

    def verify(self, capture=None, expected=None):
        data = runner.canonical(self.capture if capture is None else capture)
        return runner.capture_membership(data, runner.digest(data) if expected is None else expected,
                                         self.head, self.committed)

    def test_all_734_payloads_are_individually_bound(self):
        self.assertEqual(self.verify(), self.capture)

    def test_appended_committed_evidence_requires_its_exact_captured_bytes(self):
        path, content = ".ecdev/evidence/appended.ecg", b"new content-bound proof"
        self.committed[path] = content
        self.capture["files"].append({"path": path, "sha256": runner.digest(content), "hex": content.hex()})
        self.assertEqual(len(self.capture["files"]), 735)
        self.assertEqual(self.verify(), self.capture)
        self.capture["files"][-1]["hex"] = b"rebound".hex()
        self.capture["files"][-1]["sha256"] = runner.digest(b"rebound")
        with self.assertRaisesRegex(ValueError, "differs from committed"):
            self.verify()

    def test_same_count_payload_swap_and_omitted_original_are_refused_after_append(self):
        path, content = ".ecdev/evidence/appended.ecg", b"new content-bound proof"
        self.committed[path] = content
        appended = {"path": path, "sha256": runner.digest(content), "hex": content.hex()}
        for replacement in (appended, self.capture["files"][0]):
            changed = copy.deepcopy(self.capture)
            changed["files"].append(appended)
            changed["files"][1] = replacement
            with self.subTest(path=replacement["path"]), self.assertRaises(ValueError):
                self.verify(changed)

    def test_no_omitted_extra_duplicate_or_uncommitted_payload(self):
        for mutation in ("missing", "extra", "duplicate", "uncommitted"):
            changed = copy.deepcopy(self.capture)
            if mutation == "missing":
                changed["files"].pop()
            elif mutation == "extra":
                changed["files"].append(changed["files"][0])
            elif mutation == "duplicate":
                changed["files"][-1] = changed["files"][0]
            else:
                changed["files"][0]["path"] = ".ecdev/evidence/uncommitted.ecg"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.verify(changed)

    def test_payload_tamper_renaming_head_and_capture_pin_refuse(self):
        for mutation in ("hex", "digest", "head", "identity", "pin"):
            changed = copy.deepcopy(self.capture)
            if mutation == "hex":
                changed["files"][0]["hex"] = b"tampered".hex()
                changed["files"][0]["sha256"] = runner.digest(b"tampered")
            elif mutation == "digest":
                changed["files"][0]["sha256"] = "0" * 64
            elif mutation == "head":
                changed["observed_head"] = "b" * 40
            elif mutation == "identity":
                changed["source_id"] = "other-source"
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.verify(changed, "0" * 64 if mutation == "pin" else None)

    def test_another_committed_payload_cannot_be_omitted_from_capture(self):
        self.committed[".ecdev/history/another.ecg"] = b"retained history"
        with self.assertRaisesRegex(ValueError, "complete committed"):
            self.verify()


class QualifiedRegistry(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / "core/semantic/src/schema.rs"
        self.path.parent.mkdir(parents=True)

    def text(self, ids=SYNTHETIC_IDS):
        return "pub const SHARDS: &[Shard] = &[\n" + "".join(
            f'Shard {{ id: "{source_id}", origin: "example.invalid/{source_id}", domain: "inert", }},\n'
            for source_id in ids) + "];\n"

    def read(self, text=None, schema=None):
        self.path.write_text(self.text() if text is None else text)
        return runner.read_registered_source_ids(self.root, runner.PINNED_SCHEMA_DIGEST if schema is None else schema)

    def test_exact_qualified_grammar_reads_ordered_ids_without_writing_state(self):
        self.path.write_text(self.text())
        before = self.path.stat().st_mtime_ns
        self.assertEqual(runner.read_registered_source_ids(self.root, runner.PINNED_SCHEMA_DIGEST), SYNTHETIC_IDS)
        self.assertEqual(self.path.stat().st_mtime_ns, before)
        self.assertEqual(list(self.path.parent.iterdir()), [self.path])

    def test_wrong_schema_missing_duplicate_and_unsupported_declarations_refuse(self):
        with self.assertRaisesRegex(ValueError, "schema digest"):
            self.read(schema="sha256:" + "0" * 64)
        for text in ("", self.text() + self.text(), self.text().replace("];", ""),
                     self.text().replace("domain:", "unrecognized:"),
                     self.text().replace('domain: "inert"', "domain: unknown()")):
            with self.subTest(text=text[:50]), self.assertRaises(ValueError):
                self.read(text)

    def test_missing_extra_duplicate_or_self_registered_source_refuses(self):
        for ids in (SYNTHETIC_IDS[:-1], [*SYNTHETIC_IDS, "fixture-h"],
                    [*SYNTHETIC_IDS[:-1], SYNTHETIC_IDS[0]], [*SYNTHETIC_IDS[:-1], "ecdev"]):
            with self.subTest(ids=ids), self.assertRaisesRegex(ValueError, "seven unique"):
                self.read(self.text(ids))

class CandidateReport(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.mapping = {"target_custody": None, "node_mappings": [{"proposed_target": None}]}
        (self.root / runner.MAPPING).write_bytes(runner.canonical(self.mapping))
        (self.root / runner.INSPECTION).write_text("{}")
        self.report = {"format": "ecdev-source-eight-candidate-v1", "status": "PASS", "source_id": "ecdev",
                       "source_repository": "llgtrn/ECDEV", "capture_sha256": "sha256:" + "c" * 64,
                       "canonical_enrollment": "BLOCKED", "source_lock_installed": False,
                       "registry_binding": "PENDING", "registered_sources": 7, "source_registered": False,
                       "tests_passed": 8, "tests_failed": 0, "current_rights_authorization": None,
                       "target_mapping": None, "target_custody": None, "proposed_target": None,
                       "candidate_registry_expectation": {
                           "current_registered_source_ids": list(SYNTHETIC_IDS),
                           "required_closed_source_ids": sorted((*SYNTHETIC_IDS, "ecdev")),
                           "source_id": "ecdev", "source_repository": "llgtrn/ECDEV",
                           "canonical_source_owner": None, "registry_entry_accepted": False, "new_common_lock": None,
                       },
                       "checks": [{"name": name, "pass": True} for name in runner.CHECKS]}
        self.bind_artifacts()

    def bind_artifacts(self):
        for file, field in ((runner.MAPPING, "mapping_sha256"), (runner.INSPECTION, "inspection_sha256")):
            self.report[field] = "sha256:" + runner.digest((self.root / file).read_bytes())

    def verify(self, report=None):
        return runner.validate_candidate(self.report if report is None else report, "c" * 64, self.root, SYNTHETIC_IDS)

    def test_blocked_candidate_with_exact_eight_checks_and_artifacts_passes(self):
        self.assertEqual(set(self.verify()), {runner.MAPPING, runner.INSPECTION})

    def test_candidate_validator_requires_a_complete_supplied_registry(self):
        for ids in (None, "synthetic", SYNTHETIC_IDS[:-1], [*SYNTHETIC_IDS[:-1], "ecdev"],
                    [*SYNTHETIC_IDS[:-1], SYNTHETIC_IDS[0]]):
            with self.subTest(ids=ids), self.assertRaisesRegex(ValueError, "qualified seven-source"):
                runner.validate_candidate(self.report, "c" * 64, self.root, ids)

    def test_self_grants_wrong_identity_and_type_coercion_refuse(self):
        for field, value in (("canonical_enrollment", "ENROLLED"), ("registry_binding", "BOUND"),
                             ("source_lock_installed", True), ("source_registered", True),
                             ("current_rights_authorization", {}), ("target_mapping", {}),
                             ("target_custody", {}), ("proposed_target", {}),
                             ("source_id", "other-source"), ("registered_sources", 8),
                             ("tests_passed", True), ("tests_failed", False),
                             ("capture_sha256", "sha256:" + "0" * 64)):
            changed = copy.deepcopy(self.report)
            changed[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify(changed)

    def test_omitted_duplicate_reordered_and_failed_checks_refuse(self):
        for mutation in ("missing", "duplicate", "reordered", "failed"):
            changed = copy.deepcopy(self.report)
            if mutation == "missing":
                changed["checks"].pop()
            elif mutation == "duplicate":
                changed["checks"][-1] = changed["checks"][0]
            elif mutation == "reordered":
                changed["checks"].reverse()
            else:
                changed["checks"][0]["pass"] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.verify(changed)

    def test_registry_metadata_cannot_grant_authority_or_shrink_source_sets(self):
        registered = list(SYNTHETIC_IDS)
        required = sorted((*registered, "ecdev"))
        for field, value in (("canonical_source_owner", {"owner": "self"}),
                             ("registry_entry_accepted", True), ("registry_entry_accepted", 0),
                             ("new_common_lock", {"engine": "self"}), ("source_id", "other-source"),
                             ("source_repository", "llgtrn/other"),
                             ("current_registered_source_ids", registered[:-1]),
                             ("current_registered_source_ids", registered[:-1] + [registered[0]]),
                             ("current_registered_source_ids", registered + ["ecdev"]),
                             ("required_closed_source_ids", sorted(registered)),
                             ("required_closed_source_ids", required + ["invented"]),
                             ("required_closed_source_ids", required[:-1] + [required[0]])):
            changed = copy.deepcopy(self.report)
            changed["candidate_registry_expectation"][field] = value
            with self.subTest(field=field, value=value), self.assertRaisesRegex(ValueError, "registry expectation"):
                self.verify(changed)

    def test_registry_metadata_requires_every_exact_key_without_extra_fields(self):
        for field in self.report["candidate_registry_expectation"]:
            changed = copy.deepcopy(self.report)
            del changed["candidate_registry_expectation"][field]
            with self.subTest(missing=field), self.assertRaisesRegex(ValueError, "registry expectation"):
                self.verify(changed)
        for value in (None, [], "PENDING", {**self.report["candidate_registry_expectation"], "grant": None}):
            changed = copy.deepcopy(self.report)
            changed["candidate_registry_expectation"] = value
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "registry expectation"):
                self.verify(changed)
        changed = copy.deepcopy(self.report)
        del changed["candidate_registry_expectation"]
        with self.assertRaisesRegex(ValueError, "registry expectation"):
            self.verify(changed)

    def test_changed_artifact_and_digest_rebound_self_grant_refuse(self):
        (self.root / runner.MAPPING).write_text("tampered")
        with self.assertRaisesRegex(ValueError, "artifact digest"):
            self.verify()
        self.mapping["target_custody"] = {"self": "grant"}
        (self.root / runner.MAPPING).write_bytes(runner.canonical(self.mapping))
        self.bind_artifacts()
        with self.assertRaisesRegex(ValueError, "self-grant"):
            self.verify()

    def test_missing_mapping_fields_cannot_assert_an_empty_success(self):
        (self.root / runner.MAPPING).write_text('{"target_custody":null}')
        self.bind_artifacts()
        with self.assertRaisesRegex(ValueError, "self-grant"):
            self.verify()

    def test_json_duplicates_and_nonfinite_values_refuse(self):
        for text in ('{"status":"PASS","status":"FAIL"}', '{"unknown":NaN}'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                runner.strict_json(text)

    def test_git_environment_disables_network_helpers_prompts_and_credentials(self):
        with patch.dict(runner.os.environ, {"GH_TOKEN": "secret", "GIT_DIR": "other", "GIT_CONFIG_COUNT": "1"}):
            with patch.object(runner.subprocess, "run") as command:
                command.return_value.stdout = b"objects"
                runner.git(self.root, "cat-file", "--batch", input_bytes=b"a\n")
                args, kwargs = command.call_args
                self.assertIn("protocol.allow=never", args[0])
                self.assertIn("core.fsmonitor=false", args[0])
                self.assertEqual(kwargs["input"], b"a\n")
                for key in ("GH_TOKEN", "GIT_DIR", "GIT_CONFIG_COUNT"):
                    self.assertNotIn(key, kwargs["env"])
                for key in ("GIT_NO_LAZY_FETCH", "GIT_NO_REPLACE_OBJECTS"):
                    self.assertEqual(kwargs["env"][key], "1")
                self.assertEqual(kwargs["env"]["GIT_TERMINAL_PROMPT"], "0")
                self.assertEqual(kwargs["env"]["GIT_CONFIG_SYSTEM"], runner.os.devnull)


if __name__ == "__main__":
    unittest.main()
