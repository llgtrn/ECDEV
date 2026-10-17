"""Portable source material membership is integrity evidence, never admission."""

import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import price_source_package as package


ROOT = Path(__file__).absolute().parents[2]
PINS = {"expected_source_commit": "c973d76bda8476142dd64116b60877942ccb24d5",
        "expected_source_tree": "1020e7ebdc85befdd0e6ee4d73e4417779228f2c",
        "expected_rights_sha256": package.RIGHTS_SHA256}


class PortableSelectedSourcePackage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.value = package.build_package(ROOT, **PINS)
        cls.raw = package.encode(cls.value)
        cls.pin = package.digest(cls.raw)

    def changed(self, value):
        raw = package.encode(value)
        return package.inspect_package(raw, package.digest(raw), **PINS)

    def rejected(self, mutate):
        value = copy.deepcopy(self.value)
        mutate(value)
        with self.assertRaises(ValueError):
            self.changed(value)

    @staticmethod
    def replace_bytes(row, raw, *, kind=None):
        row.update(bytes=len(raw), content_hex=raw.hex())
        if kind:
            row["oid"] = package.git_digest(kind, raw)
        else:
            row.update(sha256=package.digest(raw), git_blob=package.git_digest("blob", raw))

    def test_exact83_files_and86_memberships_preserve_independent_pins(self):
        result = package.inspect_package(self.raw, self.pin, **PINS)
        self.assertEqual(result["material_files"], 83)
        self.assertEqual(result["material_bytes"], 4147289)
        self.assertEqual(result["evidence_files"], 3)
        self.assertEqual(result["tree_objects"], 19)
        self.assertEqual(len(self.raw), 14277535)
        self.assertEqual(self.pin, "be31c8f8067af13502194f81d2642e0eddd184f1856f3b96b64066227a9716bd")
        self.assertEqual(result["source_commit"], PINS["expected_source_commit"])
        rights = json.loads(bytes.fromhex(next(r["content_hex"] for r in self.value["evidence"]
                                              if r["path"] == package.RIGHTS_PATH)))
        self.assertEqual(result["selected_source_bindings"], rights["selected_source_bindings"])
        self.assertEqual(sorted(int(r["capture_locator"].rsplit("/", 1)[1])
                                for r in result["selected_source_bindings"].values()), [306, 503, 513, 723])

    def test_notices_are_exact_including_bom_and_all_conditions(self):
        records = {r["path"]: r for r in self.value["materials"]}
        for path, length, sha in [
                ("adapter/web/tests/fixtures/price-parser-LICENSE.txt", 1513,
                 "3b017dd7265380c41490112a1ffe70579300ce2b1deb3a37013b61ece99036c5"),
                ("adapter/web/data/price-unicode-NOTICE.txt", 2275,
                 "80b381b2f157c31e7f3222bdb932bda9fbd7b335e888beec56820fb2993363de"),
                ("adapter/web/data/price-unicode-14.0.0-ReadMe.txt", 654,
                 "7316825946e47cdfb77f6c9037c611d74d58b2e55761ea0d6dfa162fa07a926a")]:
            with self.subTest(path=path):
                row = records[path]
                self.assertEqual(row["bytes"], length)
                self.assertEqual(row["sha256"], sha)
                self.assertEqual(bytes.fromhex(row["content_hex"]), (ROOT / path).read_bytes())
        self.assertTrue(bytes.fromhex(records["adapter/web/data/price-unicode-NOTICE.txt"]
                                      ["content_hex"]).startswith(b"\xef\xbb\xbf"))

    def test_existing_readers_can_access_only_verified_original_evidence_bytes(self):
        result, evidence = package.inspect_package_evidence(self.raw, self.pin, **PINS)
        self.assertEqual(result["package_sha256"], self.pin)
        self.assertEqual(set(evidence), set(package.EVIDENCE))
        for path, sha in package.EVIDENCE.items():
            raw = package.verified_evidence(self.raw, self.pin, path=path, **PINS)
            self.assertEqual(package.digest(raw), sha)
            self.assertEqual(raw, (ROOT / path).read_bytes())
            self.assertEqual(raw, evidence[path])
        for path in ("AGENTS.md", "../file", None):
            with self.subTest(path=path), self.assertRaises(ValueError):
                package.verified_evidence(self.raw, self.pin, path=path, **PINS)
        with self.assertRaises(ValueError):
            package.verified_evidence(self.raw + b" ", package.digest(self.raw + b" "),
                                      path=package.WITNESS_PATH, **PINS)

    def test_history_and_current_rights_cannot_be_upgraded(self):
        result = package.inspect_package(self.raw, self.pin, **PINS)
        self.assertEqual(result["historical_rights_status"]["copying_authorization"], "UNKNOWN")
        self.assertIsNone(result["historical_rights_status"]["current_rights"])
        self.assertIsNone(result["published_rights_status"]["current_production_rights"])
        self.assertEqual(result["operational_authority"], package.STATUS)
        for key, fake in [("current_production_rights", {}), ("canonical_enrollment", "ADMITTED"),
                          ("target_exclusive_custody", True), ("source_adapter_cutover", True),
                          ("context_nodes_adopted", 69), ("all828_admitted_mappings", 828),
                          ("production_obligations_closed", False)]:
            with self.subTest(key=key):
                self.rejected(lambda v: v["status"].__setitem__(key, fake))
        self.rejected(lambda v: v.__setitem__("claim", "COMPLETE_RUNTIME_CLOSURE"))
        self.rejected(lambda v: v.__setitem__("authority", True))

    def test_all_independent_pins_are_required_even_with_recomputed_package_hash(self):
        for name, wrong in [("expected_source_commit", "0" * 40),
                            ("expected_source_tree", "0" * 40),
                            ("expected_rights_sha256", "0" * 64)]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                package.inspect_package(self.raw, self.pin, **{**PINS, name: wrong})
        for fake in (None, False, "", "0" * 64, self.pin.upper(), "sha256:" + self.pin):
            with self.subTest(fake=fake), self.assertRaises(ValueError):
                package.inspect_package(self.raw, fake, **PINS)
        for key in PINS:
            with self.subTest(key=key), self.assertRaises(TypeError):
                package.inspect_package(self.raw, self.pin, **{k: v for k, v in PINS.items() if k != key})
        self.rejected(lambda v: v.__setitem__("source_commit", "0" * 40))
        self.rejected(lambda v: v.__setitem__("source_tree", "0" * 40))

    def test_mutated_commit_cannot_replace_the_independent_commit_anchor(self):
        for rehash in (False, True):
            def change(value):
                row = value["commit"]
                raw = bytes.fromhex(row["content_hex"]) + b"changed\n"
                if rehash:
                    self.replace_bytes(row, raw, kind="commit")
                    value["source_commit"] = row["oid"]
                else:
                    row.update(bytes=len(raw), content_hex=raw.hex())
            with self.subTest(rehash=rehash):
                self.rejected(change)

    def test_mutated_raw_tree_path_mode_and_blob_cannot_reanchor_membership(self):
        for replacement in (b"100755 AGENTS.md\0", b"100644 BGENTS.md\0",
                            b"120000 AGENTS.md\0"):
            for rehash in (False, True):
                def change(value):
                    row = next(r for r in value["trees"] if r["oid"] == value["source_tree"])
                    raw = bytes.fromhex(row["content_hex"])
                    self.assertIn(b"100644 AGENTS.md\0", raw)
                    raw = raw.replace(b"100644 AGENTS.md\0", replacement, 1)
                    if rehash:
                        self.replace_bytes(row, raw, kind="tree")
                    else:
                        row.update(bytes=len(raw), content_hex=raw.hex())
                with self.subTest(replacement=replacement, rehash=rehash):
                    self.rejected(change)
        def change_blob(value):
            row = next(r for r in value["trees"] if r["oid"] == value["source_tree"])
            raw = bytes.fromhex(row["content_hex"])
            start = raw.index(b"100644 AGENTS.md\0") + len(b"100644 AGENTS.md\0")
            raw = raw[:start] + b"\0" * 20 + raw[start + 20:]
            self.replace_bytes(row, raw, kind="tree")
        self.rejected(change_blob)

    def test_material_and_notice_rehashes_do_not_bypass_published83_pins(self):
        for path in ("AGENTS.md", "adapter/web/data/price-unicode-NOTICE.txt",
                     "adapter/web/data/price-unicode-14.0.0-ReadMe.txt",
                     "adapter/web/tests/fixtures/price-parser-LICENSE.txt"):
            for rehash in (False, True):
                def change(value):
                    row = next(r for r in value["materials"] if r["path"] == path)
                    raw = bytes.fromhex(row["content_hex"]) + b"x"
                    if rehash:
                        self.replace_bytes(row, raw)
                    else:
                        row.update(bytes=len(raw), content_hex=raw.hex())
                with self.subTest(path=path, rehash=rehash):
                    self.rejected(change)
        self.rejected(lambda v: v["materials"][0].__setitem__("mode", "100755"))

    def test_immutable_evidence_cannot_be_replaced_with_new_self_pins(self):
        for path in package.EVIDENCE:
            def change(value):
                row = next(r for r in value["evidence"] if r["path"] == path)
                raw = bytes.fromhex(row["content_hex"]) + b" "
                self.replace_bytes(row, raw)
            with self.subTest(path=path):
                self.rejected(change)

    def test_missing_extra_duplicate_unsorted_and_dangling_records_fail(self):
        for name in ("materials", "evidence", "trees"):
            for operation in ("missing", "extra", "duplicate", "reverse"):
                def change(value):
                    rows = value[name]
                    if operation == "missing":
                        rows.pop()
                    elif operation == "extra":
                        rows.append(copy.deepcopy(rows[0]))
                    elif operation == "duplicate":
                        rows[-1] = copy.deepcopy(rows[0])
                    else:
                        rows.reverse()
                with self.subTest(name=name, operation=operation):
                    self.rejected(change)
        def dangling(value):
            row = package.object_record(package.git_digest("tree", b""), b"")
            value["trees"].append(row)
            value["trees"].sort(key=lambda r: r["oid"])
        self.rejected(dangling)
        self.rejected(lambda v: v["materials"][0].__setitem__("extra", None))

    def test_bool_float_negative_and_over_budget_counts_and_bad_hex_fail(self):
        for section in ("commit", "trees", "materials", "evidence"):
            for fake in (False, True, 0.0, -1, package.MAX_FILE_BYTES + 1, "1", None):
                def change(value):
                    row = value[section] if section == "commit" else value[section][0]
                    row["bytes"] = fake
                with self.subTest(section=section, fake=fake):
                    self.rejected(change)
        for fake in ("zz", "ab ", "AB", None, 0):
            with self.subTest(fake=fake):
                self.rejected(lambda v: v["materials"][0].__setitem__("content_hex", fake))
        self.rejected(lambda v: v.__setitem__("trees", [v["trees"][0]] * (package.MAX_TREES + 1)))
        with mock.patch.object(package, "MAX_TOTAL_BYTES", 100), self.assertRaises(ValueError):
            package.inspect_package(self.raw, self.pin, **PINS)
        with mock.patch.object(package, "MAX_PACKAGE_BYTES", 100), self.assertRaises(ValueError):
            package.inspect_package(self.raw, self.pin, **PINS)
        with mock.patch.object(package, "MAX_TREE_ENTRIES", 1), self.assertRaises(ValueError):
            package.inspect_package(self.raw, self.pin, **PINS)

    def test_unsafe_paths_and_nonregular_modes_fail(self):
        for fake in ("/tmp/x", "../x", "a/../x", "a//x", "./x", "a\\x", "C:/x", "a\0x",
                     "a/", "\ud800", None, False):
            with self.subTest(fake=repr(fake)):
                self.rejected(lambda v: v["materials"][0].__setitem__("path", fake))
        for fake in ("120000", "160000", "40000", 100644, True):
            with self.subTest(fake=fake):
                self.rejected(lambda v: v["materials"][0].__setitem__("mode", fake))

    def test_strict_json_duplicate_nonfinite_trailing_and_depth_node_limits(self):
        for raw in (self.raw + b"garbage", self.raw + b" ",
                    self.raw.replace(b'"format":', b'"format": null, "format":', 1),
                    b'{"x":NaN}', b'{"x":Infinity}', b'{"x":-Infinity}', b"\xff"):
            with self.subTest(prefix=raw[:30]), self.assertRaises(ValueError):
                package.inspect_package(raw, package.digest(raw), **PINS)
        with self.assertRaises(ValueError):
            package.strict_json(b'{"x":1e999}')
        with self.assertRaises(ValueError):
            package.strict_json(b"[" * 66 + b"0" + b"]" * 66)
        with mock.patch.object(package, "MAX_JSON_NODES", 4), self.assertRaises(ValueError):
            package.strict_json(b"[0,1,2,3]")
        with self.assertRaises(ValueError):
            package.strict_json(b"[" * 2000 + b"0" + b"]" * 2000)

    def test_raw_tree_parser_rejects_ambiguous_or_dangerous_structure(self):
        oid = b"\x11" * 20
        for raw in (b"100644 x", b"100644 x\0" + oid[:-1], b"100600 x\0" + oid,
                    b"100644 ../x\0" + oid, b"100644 x\\y\0" + oid,
                    b"100644 x\0" + oid + b"100644 x\0" + oid,
                    b"100644 z\0" + oid + b"100644 a\0" + oid):
            with self.subTest(raw=raw[:30]), self.assertRaises(ValueError):
                package.parse_tree(raw)
        trees = {"a": {b"x": ("120000", "b")}}
        with self.assertRaises(ValueError):
            package.membership("x/child", trees, "a", set())
        with self.assertRaises(ValueError):
            package.membership("x", trees, "a", set())

    def test_builder_refuses_changed_material_and_evidence_without_source_writes(self):
        original = package.regular
        for path in ("AGENTS.md", package.RIGHTS_PATH, package.WITNESS_PATH, package.PREREQUISITE_PATH):
            def changed(path_arg, maximum):
                raw = original(path_arg, maximum)
                return raw + b"x" if Path(path_arg) == ROOT / path else raw
            with self.subTest(path=path), mock.patch.object(package, "regular", side_effect=changed), \
                    self.assertRaises(ValueError):
                package.build_package(ROOT, **PINS)

    def test_export_is_repeatable_and_reads_source_only(self):
        paths = [ROOT / r["path"] for kind in ("materials", "evidence") for r in self.value[kind]]
        before = [(p.stat().st_mode, p.stat().st_mtime_ns, hashlib.sha256(p.read_bytes()).hexdigest())
                  for p in paths]
        again = package.encode(package.build_package(ROOT, **PINS))
        self.assertEqual(again, self.raw)
        self.assertEqual(before, [(p.stat().st_mode, p.stat().st_mtime_ns,
                                   hashlib.sha256(p.read_bytes()).hexdigest()) for p in paths])

    def test_builder_refuses_worktree_executable_mode_drift_for_material_and_evidence(self):
        original = package.worktree_mode
        for path in ("AGENTS.md", package.RIGHTS_PATH):
            def changed(path_arg):
                mode = original(path_arg)
                return ("100755" if mode == "100644" else "100644") \
                    if Path(path_arg) == ROOT / path else mode
            with self.subTest(path=path), mock.patch.object(package, "worktree_mode", side_effect=changed), \
                    self.assertRaisesRegex(ValueError, "mode changed"):
                package.build_package(ROOT, **PINS)

    def test_builder_rechecks_all86_paths_and_refuses_mid_export_byte_or_mode_changes(self):
        original_read, original_mode = package.regular, package.worktree_mode
        for path in ("AGENTS.md", package.RIGHTS_PATH):
            for kind in ("bytes", "mode"):
                counts = {}
                def changed_read(path_arg, maximum):
                    raw = original_read(path_arg, maximum)
                    counts[str(path_arg)] = counts.get(str(path_arg), 0) + 1
                    return raw + b"x" if kind == "bytes" and Path(path_arg) == ROOT / path \
                        and counts[str(path_arg)] == 2 else raw
                mode_calls = {}
                def changed_mode(path_arg):
                    mode = original_mode(path_arg)
                    mode_calls[str(path_arg)] = mode_calls.get(str(path_arg), 0) + 1
                    return ("100755" if mode == "100644" else "100644") \
                        if kind == "mode" and Path(path_arg) == ROOT / path \
                        and mode_calls[str(path_arg)] == 2 else mode
                with self.subTest(path=path, kind=kind), \
                        mock.patch.object(package, "regular", side_effect=changed_read), \
                        mock.patch.object(package, "worktree_mode", side_effect=changed_mode), \
                        self.assertRaisesRegex(ValueError, "changed during export"):
                    package.build_package(ROOT, **PINS)
        counts = {}
        def observe(path_arg, maximum):
            counts[str(path_arg)] = counts.get(str(path_arg), 0) + 1
            return original_read(path_arg, maximum)
        with mock.patch.object(package, "regular", side_effect=observe):
            package.build_package(ROOT, **PINS)
        self.assertEqual(len(counts), 86)
        self.assertEqual(set(counts.values()), {2})

    def test_inspector_has_no_git_checkout_root_or_other_module_dependency(self):
        with mock.patch.object(package, "git_object", side_effect=AssertionError("Git called")), \
                mock.patch.object(package, "regular", side_effect=AssertionError("file read")), \
                mock.patch.object(subprocess, "run", side_effect=AssertionError("execution")):
            self.assertFalse(package.inspect_package(self.raw, self.pin, **PINS)
                             ["offline_git_or_checkout_required"])
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            script, exported = base / "reader.py", base / "selected.json"
            shutil.copyfile(Path(package.__file__), script)
            exported.write_bytes(self.raw)
            command = [sys.executable, "-I", "-B", str(script), "inspect", "--package", str(exported),
                       "--expect-package-sha256", self.pin, "--expect-source-commit",
                       PINS["expected_source_commit"], "--expect-source-tree", PINS["expected_source_tree"],
                       "--expect-rights-sha256", package.RIGHTS_SHA256]
            result = subprocess.run(command, cwd=base, env={"PATH": str(base)}, capture_output=True,
                                    timeout=30, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            inspected = json.loads(result.stdout)
            self.assertEqual(inspected["material_files"], 83)
            self.assertFalse(inspected["offline_git_or_checkout_required"])

    def test_existing_output_is_never_overwritten_and_symlinks_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "out.json"
            package.write_new_or_equal(path, b"same")
            before = path.stat().st_mtime_ns
            package.write_new_or_equal(path, b"same")
            self.assertEqual(path.stat().st_mtime_ns, before)
            with self.assertRaises(ValueError):
                package.write_new_or_equal(path, b"changed")
            self.assertEqual(path.read_bytes(), b"same")
            link = root / "link"
            link.symlink_to(path)
            with self.assertRaises(ValueError):
                package.write_new_or_equal(link, b"same")
            folder = root / "folder"
            folder.symlink_to(root, target_is_directory=True)
            with self.assertRaises(ValueError):
                package.write_new_or_equal(folder / "new", b"same")
            fifo = root / "fifo"
            os.mkfifo(fifo)
            with self.assertRaises(ValueError):
                package.regular(fifo, 100)

    def test_cli_requires_pins_and_required_pins_are_labeled_untrusted(self):
        with tempfile.TemporaryDirectory() as directory:
            exported = Path(directory) / "package.json"
            exported.write_bytes(self.raw)
            command = [sys.executable, "-B", package.__file__]
            result = subprocess.run([*command, "inspect", "--package", str(exported)],
                                    capture_output=True, timeout=30, check=False)
            self.assertNotEqual(result.returncode, 0)
            result = subprocess.run([*command, "required-pins", "--package", str(exported),
                                     "--expect-package-sha256", self.pin],
                                    capture_output=True, timeout=30, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("INFORMATION_ONLY", json.loads(result.stdout)["warning"])

    def test_git_reader_disables_replacements_fetch_hooks_and_external_config(self):
        original = subprocess.run
        calls = []
        def observe(args, **kwargs):
            calls.append((args, kwargs["env"]))
            return original(args, **kwargs)
        with mock.patch.object(subprocess, "run", side_effect=observe):
            package.git_object(ROOT, "commit", PINS["expected_source_commit"])
        self.assertEqual(len(calls), 2)
        for args, env in calls:
            self.assertIn("--no-replace-objects", args)
            self.assertIn("protocol.allow=never", args)
            self.assertIn("core.hooksPath=" + os.devnull, args)
            self.assertEqual(env["GIT_NO_LAZY_FETCH"], "1")
            self.assertEqual(env["GIT_CONFIG_GLOBAL"], os.devnull)
            self.assertEqual(env["GIT_CONFIG_NOSYSTEM"], "1")


if __name__ == "__main__":
    unittest.main()
