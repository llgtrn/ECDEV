"""Bounded refusal tests for the source qualification runner; no engine execution."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("qualification_runner", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class QualificationBoundaries(unittest.TestCase):
    def test_git_reads_disable_lazy_fetch_prompts_and_external_helpers(self):
        with patch.dict(runner.os.environ, {"GIT_DIR": "unexpected", "GH_TOKEN": "must-not-enter-child"}):
            with patch.object(runner.subprocess, "run") as command:
                command.return_value.stdout = b"tree"
                self.assertEqual(runner.git(Path("/objects"), "rev-parse", "pin^{tree}"), b"tree")
                args, kwargs = command.call_args
                self.assertIn("protocol.allow=never", args[0])
                self.assertIn("core.fsmonitor=false", args[0])
                self.assertEqual(kwargs["env"]["GIT_NO_LAZY_FETCH"], "1")
                self.assertEqual(kwargs["env"]["GIT_TERMINAL_PROMPT"], "0")
                self.assertNotIn("GH_TOKEN", kwargs["env"])
                self.assertNotIn("GIT_DIR", kwargs["env"])

    def test_report_must_match_reviewed_engine_fixture_and_pending_contract(self):
        contract = {"engine_commit": "a" * 40, "protocol_version": 1, "compatibility_epoch": 1,
                    "schema_digest": "schema", "engine_digest": "engine", "fixture_sha256": "b" * 64}
        report = {"format": "ecdev-central-intake-qualification-v1", "status": "PASS",
                  "engine_commit": contract["engine_commit"], "protocol_version": 1,
                  "compatibility_epoch": 1, "central_schema": "schema", "central_engine": "engine",
                  "registered_sources": 7, "source_registered": False, "tests_failed": 0,
                  "fixture": {"registry_binding": "PENDING", "sha256": "sha256:" + contract["fixture_sha256"]},
                  "fresh": {"registry_binding": "PENDING"}, "checks": [{"pass": True}],
                  "reader_probes": [], "tests_passed": 1}
        runner.validate_report(report, contract)
        for field, value in (("engine_commit", "different"), ("central_schema", "different"),
                             ("source_registered", True), ("status", "FAIL"), ("protocol_version", True),
                             ("tests_passed", 0), ("checks", [{"pass": False}]),
                             ("fresh", {"registry_binding": "ENROLLED"}),
                             ("fixture", {"registry_binding": "PENDING", "sha256": "wrong"})):
            changed = json.loads(json.dumps(report))
            changed[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.validate_report(changed, contract)

    def test_build_environment_drops_credentials_and_compiler_wrappers_and_refuses_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            project = root / "project"
            project.mkdir()
            home = root / "cache"
            home.mkdir()
            with patch.dict(runner.os.environ, {"CARGO_HOME": str(home), "GH_TOKEN": "must-not-enter-child",
                                                "RUSTC_WRAPPER": "unexpected", "CARGO_ENCODED_RUSTFLAGS": "unexpected"}):
                env = runner.build_environment(project, root)
                self.assertNotIn("GH_TOKEN", env)
                self.assertNotIn("RUSTC_WRAPPER", env)
                self.assertNotIn("CARGO_ENCODED_RUSTFLAGS", env)
                self.assertEqual(env["CARGO_NET_OFFLINE"], "true")
                self.assertEqual(env["GIT_NO_LAZY_FETCH"], "1")
                self.assertEqual(env["GIT_TERMINAL_PROMPT"], "0")
                self.assertEqual(env["GIT_CONFIG_KEY_0"], "protocol.allow")
                self.assertEqual(env["GIT_CONFIG_VALUE_0"], "never")
                (project / ".cargo").mkdir()
                (project / ".cargo/config.toml").write_text("[build]\nrustc-wrapper='unexpected'\n")
                with self.assertRaisesRegex(ValueError, "unbound Cargo"):
                    runner.build_environment(project, root)

    def test_output_is_new_and_disjoint_in_both_directions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, engine, objects = (root / name for name in ("source", "engine", "objects"))
            for path in (source, engine, objects):
                path.mkdir()
            for output in (source, source / "out", engine / "out", objects / "out", root):
                with self.subTest(output=output), self.assertRaises(ValueError):
                    runner.separate_output(output, (source, engine, objects))
            runner.separate_output(root / "new", (source, engine, objects))
            (root / "existing").mkdir()
            with self.assertRaises(ValueError):
                runner.separate_output(root / "existing", (source, engine, objects))
            (root / "link").symlink_to(source, target_is_directory=True)
            with self.assertRaises(ValueError):
                runner.separate_output((root / "link/out").resolve(), (source, engine, objects))
            for path in (root / "link/out", root / "dangling"):
                if path.name == "dangling":
                    path.symlink_to(root / "nonexistent-output")
                with self.assertRaisesRegex(ValueError, "symlink"):
                    runner.physical_path(path)
            with self.assertRaisesRegex(ValueError, "parent traversal"):
                runner.physical_path(root / "new/../elsewhere")

    def test_all_pinned_bytes_and_modes_are_checked_without_modifying_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = b"reader source\n"
            path = root / "reader.rs"
            path.write_bytes(data)
            path.chmod(0o644)
            oid = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
            contract = {"engine_commit": "a" * 40, "engine_tree": "b" * 40}

            def git(_objects, command, *args):
                if command == "rev-parse":
                    return (contract["engine_tree"] + "\n").encode()
                return f"100644 blob {oid}\treader.rs\0".encode()

            with patch.object(runner, "git", side_effect=git):
                before = path.stat()
                self.assertEqual(runner.verify_reference(root, root, contract)["files"], 1)
                self.assertEqual(path.read_bytes(), data)
                self.assertEqual(path.stat().st_mtime_ns, before.st_mtime_ns)
                for changed in (b"changed source\n", b""):
                    path.write_bytes(changed)
                    with self.assertRaisesRegex(ValueError, "changed"):
                        runner.verify_reference(root, root, contract)
                path.write_bytes(data)
                path.chmod(0o755)
                with self.assertRaisesRegex(ValueError, "changed"):
                    runner.verify_reference(root, root, contract)
                path.chmod(0o644)
                (root / "extra").write_text("unreviewed build input")
                with self.assertRaisesRegex(ValueError, "extra"):
                    runner.verify_reference(root, root, contract)
                (root / "extra").unlink()
                path.unlink()
                with self.assertRaisesRegex(ValueError, "missing"):
                    runner.verify_reference(root, root, contract)
                path.symlink_to("missing-target")
                with self.assertRaisesRegex(ValueError, "nonregular"):
                    runner.verify_reference(root, root, contract)

    def test_commit_tree_and_unsupported_tree_entries_refuse_before_build(self):
        contract = {"engine_commit": "a" * 40, "engine_tree": "b" * 40}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(runner, "git", return_value=b"wrong-tree\n"):
                with self.assertRaisesRegex(ValueError, "commit/tree"):
                    runner.verify_reference(root, root, contract)
            for entry in (b"120000 blob c\tlink\0", b"160000 commit c\tsubmodule\0", b"100644 blob c\t../escape\0"):
                with patch.object(runner, "git", side_effect=[(contract["engine_tree"] + "\n").encode(), entry]):
                    with self.assertRaises(ValueError):
                        runner.verify_reference(root, root, contract)


if __name__ == "__main__":
    unittest.main()
