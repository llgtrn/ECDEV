"""Exact retained-data and refusal tests for the bounded EUR parser extension."""

from contextlib import ExitStack, contextmanager
import copy
import hashlib
import importlib.util
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
QUALIFICATION = Path(".ecdev/qualification")
DATA = Path("research/commerce/governance")
PINS = {
    "price-eur-target-handoff-v1.json": "9657f611bee636dc08613eb3fe68511424afba764b474aed53fe1243c4a7fc20",
    "price-eur-target-case-map-v1.jsonl": "b8caf5500745de277cfb9d011722b05e884a0e8e76671e0e849756720f8cd16d",
    "price-eur-target-membership-v1.json": "2c1ca71af5a29d093ff2dbc1a325156a449c9bf1d8cf9ed2aa46f3c0891e675a",
    "price-eur-pure-probe-v1.json": "fe3a4236412946e90cd2416fea3ca8db0ac2be175be4ec64297d7b7d5f8235b2",
}
NEW = [117, 212, 409, 855, 1188, 1193, 1849]
NEW_MANIFEST = "5beaf6ab248dde7a3c42e6ef1b177c64e8c3af481561e5dd2d76aa2d893d342e"
ALL_MANIFEST = "451f0bb1a2f3acd3879ddc2945c14e74447ec6ac70dfd5ebf374d086c8f353dc"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def digest(data):
    return hashlib.sha256(data).hexdigest()


def git_object(kind, body):
    return hashlib.sha1(kind.encode() + b" " + str(len(body)).encode() + b"\0" + body).hexdigest()


def git_tree(entries):
    ordered = sorted(entries, key=lambda row: row["name"].encode() + (b"/" if row["type"] == "tree" else b""))
    body = b"".join(row["mode"].lstrip("0").encode() + b" " + row["name"].encode()
                    + b"\0" + bytes.fromhex(row["oid"]) for row in ordered)
    return git_object("tree", body)


def wire_string(value):
    raw = value.encode()
    return struct.pack("<I", len(raw)) + raw


def wire_optional(value):
    return b"\0" if value is None else b"\1" + wire_string(value)


def request_hash(case, domain):
    return digest(wire_string(domain) + b"".join(wire_optional(case[key]) for key in
                  ("input", "currency_hint", "decimal_separator", "digit_group_separator")))


def projection_hash(fields, domain):
    wire = wire_string(domain) + b"".join(wire_optional(fields[key]) for key in ("amount", "currency", "amount_text"))
    if "amount_float_bits" in fields:
        bits = fields["amount_float_bits"]
        number = None if bits is None else bytes.fromhex(bits)[::-1]
    else:
        number = None if fields["amount_float"] is None else struct.pack("<d", fields["amount_float"])
    return digest(wire + (b"\0" if number is None else b"\1" + number))


class RetainedEUR(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.helper = load_module("price_eur", ROOT / QUALIFICATION / "price_eur.py")
        cls.core = load_module("price_subset", ROOT / QUALIFICATION / "price_subset.py")
        cls.old = load_module("price_handoff", ROOT / QUALIFICATION / "price_handoff.py")
        for relative, expected in PINS.items():
            if digest((ROOT / DATA / relative).read_bytes()) != expected:
                raise ValueError("retained extension artifact changed: " + relative)
        cls.corpus, _ = cls.core.load_source(ROOT)
        cls.oldresults = cls.core.parse_json((ROOT / cls.old.RESULTS_PATH).read_bytes())
        cls.newresults = cls.core.parse_json((ROOT / DATA / "price-eur-pure-probe-v1.json").read_bytes())
        cls.oldrows = [cls.core.parse_json(line) for line in (ROOT / cls.old.MAP_PATH).read_bytes().splitlines()]
        cls.newrows = [cls.core.parse_json(line) for line in
                       (ROOT / DATA / "price-eur-target-case-map-v1.jsonl").read_bytes().splitlines()]
        cls.handoff = cls.core.parse_json((ROOT / DATA / "price-eur-target-handoff-v1.json").read_bytes())
        cls.witness = cls.core.parse_json((ROOT / DATA / "price-eur-target-membership-v1.json").read_bytes())
        mapping = cls.handoff["source_case_mapping"]
        cls.request_domain = re.search(r"str\(([^)]+)\)", mapping["request_digest"]).group(1)
        cls.projection_domain = re.search(r"str\(([^)]+)\)", mapping["projection_digest"]).group(1)

    def validate(self, **changes):
        values = {"corpus": self.corpus, "oldresults": self.oldresults, "newresults": self.newresults,
                  "oldrows": self.oldrows, "newrows": self.newrows, "handoff": self.handoff, "witness": self.witness}
        values.update(changes)
        return self.helper.validate(**values)

    @contextmanager
    def source_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = set(self.core.SOURCE_PINS) | set(self.old.PINS) | set(self.helper.PINS) | {
                str(QUALIFICATION / name) for name in ("price_subset.py", "price_handoff.py", "price_eur.py")}
            for relative in paths:
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, target)
            with patch.object(self.helper, "__file__", str(root / QUALIFICATION / "price_eur.py")):
                yield root, paths

    @contextmanager
    def refused_runtime_fixture(self):
        """Inert local binding isolates failure paths; no successful receipt is allowed."""
        with self.source_fixture() as (root, _paths), tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
            io_root = Path(temporary)
            (io_root / "reference").mkdir()
            reference = io_root / "reference/value.rs"
            reference.write_bytes(b"inert failure-path fixture\n")
            compiler = io_root / "compiler"
            compiler.write_bytes(b"inert executable fixture\n")
            compiler.chmod(0o755)
            args, snapshot = self.helper.load_inputs(root)
            checked = self.validate()
            args = copy.deepcopy(args)
            member = next(row for row in args[-1]["files"]
                          if row["path"] == args[-2]["target_binding"]["artifact"]["path"])
            body = reference.read_bytes()
            member.update(bytes=len(body), sha256=digest(body), git_blob=git_object("blob", body), mode="100644")
            # Only the artificial I/O binding is substituted. Every caller below
            # must refuse, and ordinary data tests exercise the real sealed checks.
            stack.enter_context(patch.object(self.helper, "load_inputs", return_value=(args, snapshot)))
            stack.enter_context(patch.object(self.helper, "validate", return_value=checked))
            yield root, reference, compiler, io_root

    def test_all_retained_requests_projections_and_exact_four_fields(self):
        self.validate()
        supported = []
        for index, (case, oldrow, row, result) in enumerate(zip(
                self.corpus["cases"], self.oldrows, self.newrows, self.newresults)):
            self.assertEqual(row["index"], index)
            self.assertEqual(row["request_sha256"], request_hash(case, self.request_domain), index)
            self.assertEqual(row["source_projection_sha256"], projection_hash(case["expected"], self.projection_domain), index)
            self.assertEqual(row["request_sha256"], oldrow["request_sha256"], index)
            self.assertEqual(row["source_projection_sha256"], oldrow["source_projection_sha256"], index)
            if result["status"] == "OK":
                self.assertEqual(row["native"], "SUPPORTED", index)
                self.assertIsNone(case["error"], index)
                self.assertEqual(row["result_sha256"], projection_hash(result, self.projection_domain), index)
                self.assertEqual(row["result_sha256"], row["source_projection_sha256"], index)
                for field in ("amount", "currency", "amount_text"):
                    self.assertEqual(result[field], case["expected"][field], (index, field))
                value = case["expected"]["amount_float"]
                self.assertEqual(result["amount_float_bits"], None if value is None else struct.pack(">d", value).hex(), index)
                supported.append(index)
            else:
                self.assertEqual(row["native"], "UNMAPPED", index)
                self.assertIs(type(row["unavailable"]), int)
                self.assertEqual(row["unavailable"], result["code"], index)
        self.assertEqual(len(self.newrows), 2937)
        self.assertEqual(len(supported), 106)
        self.assertEqual(supported, self.handoff["source_case_mapping"]["supported_indices"])
        manifest = "".join(f"{i}\t{self.corpus['cases'][i]['name']}\n" for i in supported).encode()
        self.assertEqual(digest(manifest), ALL_MANIFEST)

    def test_prior_99_remain_exact_and_only_seven_cases_gain_support(self):
        old_supported = [r["index"] for r in self.oldresults if r["status"] == "OK"]
        new_supported = [r["index"] for r in self.newresults if r["status"] == "OK"]
        self.assertEqual(len(old_supported), 99)
        self.assertEqual(sorted(set(new_supported) - set(old_supported)), NEW)
        self.assertTrue(set(old_supported) <= set(new_supported))
        for index in old_supported:
            self.assertEqual(self.oldresults[index], self.newresults[index], index)
            self.assertEqual(self.oldrows[index], self.newrows[index], index)
        manifest = "".join(f"{i}\t{self.corpus['cases'][i]['name']}\n" for i in NEW).encode()
        self.assertEqual(digest(manifest), NEW_MANIFEST)
        self.assertEqual(self.corpus["cases"][409]["input"], None)
        self.assertEqual(self.corpus["cases"][1193]["input"], "")
        self.assertNotEqual(self.newrows[409]["request_sha256"], self.newrows[1193]["request_sha256"])
        for index in NEW:
            case = self.corpus["cases"][index]
            self.assertEqual(case["currency_hint"], "EUR")
            self.assertIsNone(case["decimal_separator"])
            self.assertIsNone(case["digit_group_separator"])

    def test_27_changed_refusal_codes_remain_typed_refusals(self):
        changed = []
        for index, (before, after) in enumerate(zip(self.oldresults, self.newresults)):
            if before["status"] == after["status"] == "ERR" and before != after:
                changed.append({"index": index, "from": before["code"], "to": after["code"]})
                self.assertEqual(self.newrows[index]["native"], "UNMAPPED")
            elif index not in NEW:
                self.assertEqual(before, after, index)
        self.assertEqual(changed, self.handoff["source_case_mapping"]["reason_only_EUR_changes"])
        self.assertEqual(len(changed), 27)
        self.assertEqual(sum(row["from"] == 5 and row["to"] == 8 for row in changed), 26)
        self.assertEqual([row for row in changed if row["to"] != 8], [{"index": 1253, "from": 5, "to": 11}])
        self.assertEqual(sum(row["native"] == "UNMAPPED" for row in self.newrows), 2831)

    def test_commit_and_compact_tree_witness_bind_exact_three_artifacts(self):
        self.helper.validate_witness(self.witness, self.handoff)
        self.assertEqual(git_object("commit", self.witness["commit_preimage_utf8"].encode()), self.witness["commit"])
        self.assertIn("tree " + self.witness["tree"] + "\n", self.witness["commit_preimage_utf8"])
        witnesses = {row["path"]: row for row in self.witness["tree_witness"]}
        self.assertEqual(len(witnesses), 9)
        self.assertEqual(sum(len(row["entries"]) for row in witnesses.values()), 109)
        self.assertEqual(witnesses[""]["oid"], self.witness["tree"])
        for path, row in witnesses.items():
            self.assertEqual(git_tree(row["entries"]), row["oid"], path)
            if path:
                parent, _, name = path.rpartition("/")
                self.assertIn({"name": name, "oid": row["oid"], "mode": "040000", "type": "tree"},
                              witnesses[parent]["entries"])
        self.assertEqual(len(self.witness["files"]), 3)
        for row in self.witness["files"]:
            parent, _, name = row["path"].rpartition("/")
            self.assertIn({"name": name, "oid": row["git_blob"], "mode": row["mode"], "type": "blob"},
                          witnesses[parent]["entries"])
        for filename in ("price-eur-target-handoff-v1.json", "price-eur-target-case-map-v1.jsonl"):
            raw = (ROOT / DATA / filename).read_bytes()
            matching = [row for row in self.witness["files"] if row["sha256"] == digest(raw)]
            self.assertEqual(len(matching), 1)
            self.assertEqual(git_object("blob", raw), matching[0]["git_blob"])
            self.assertEqual(len(raw), matching[0]["bytes"])
        self.assertIsNone(self.handoff["publication"]["commit"])
        self.assertIsNone(self.handoff["publication"]["tree"])
        self.assertIs(self.witness["unsigned_content_address_only"], True)
        self.assertIs(self.witness["verification"]["verified"], False)

    def test_complete_transcripts_refuse_wrong_bits_fields_types_and_false_success(self):
        index = next(i for i in NEW if self.newresults[i]["amount_float_bits"] is not None)
        unsupported = 1253
        mutations = []
        for case_index, key, value in ((index, "amount_float_bits", "0000000000000000"),
                                       (index, "currency", "USD"), (index, "amount", None),
                                       (index, "amount_text", ""), (index, "index", True),
                                       (1253, "code", True), (1253, "code", 8),
                                       (index, "fallback", "source"), (index, "admitted", True)):
            rows = copy.deepcopy(self.newresults)
            rows[case_index][key] = value
            mutations.append((f"{case_index}:{key}", rows))
        rows = copy.deepcopy(self.newresults)
        rows[unsupported] = {**copy.deepcopy(rows[index]), "index": unsupported}
        mutations.append(("invented unsupported success", rows))
        mutations.extend((("missing", self.newresults[:-1]), ("extra", self.newresults + [self.newresults[-1]])))
        for label, rows in mutations:
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate(newresults=rows)

    def test_map_refuses_missing_duplicate_reordered_shapes_hashes_codes_and_fallback(self):
        mutations = [("missing", self.newrows[:-1]), ("extra", self.newrows + [self.newrows[-1]])]
        rows = copy.deepcopy(self.newrows)
        rows[0], rows[1] = rows[1], rows[0]
        mutations.append(("reordered", rows))
        for index, key, value in ((0, "index", True), (1, "index", 0), (0, "request_sha256", "0" * 64),
                                  (0, "source_projection_sha256", "0" * 64), (117, "result_sha256", "0" * 64),
                                  (117, "native", "UNMAPPED"), (1253, "unavailable", True),
                                  (1253, "unavailable", 8), (1253, "native", "SUPPORTED"),
                                  (117, "fallback", "source"), (117, "admitted", True)):
            rows = copy.deepcopy(self.newrows)
            rows[index][key] = value
            mutations.append((f"{index}:{key}", rows))
        for label, rows in mutations:
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate(newrows=rows)

    def test_source_selectors_presence_errors_and_expected_fields_cannot_be_rebound(self):
        for index, key, value in ((117, "input", "2801"), (117, "currency_hint", ""),
                                  (117, "decimal_separator", ""), (117, "digit_group_separator", ""),
                                  (409, "input", ""), (1193, "input", None), (117, "error", "invented")):
            corpus = copy.deepcopy(self.corpus)
            corpus["cases"][index][key] = value
            with self.subTest(index=index, key=key), self.assertRaises(ValueError):
                self.validate(corpus=corpus)
        for key, value in (("amount", "2801"), ("amount_text", None), ("currency", "USD"),
                           ("amount_float", 2801.0), ("amount_float", True)):
            corpus = copy.deepcopy(self.corpus)
            corpus["cases"][117]["expected"][key] = value
            with self.subTest(expected=key, value=value), self.assertRaises(ValueError):
                self.validate(corpus=corpus)

    def test_old_transcript_and_row_custody_cannot_be_rewritten_for_new_results(self):
        rows = copy.deepcopy(self.oldresults)
        rows[117] = copy.deepcopy(self.newresults[117])
        with self.assertRaises(ValueError):
            self.validate(oldresults=rows)
        for key, value in (("request_sha256", "0" * 64), ("unavailable", 8), ("index", True)):
            rows = copy.deepcopy(self.oldrows)
            rows[117][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.validate(oldrows=rows)

    def test_sealed_handoff_refuses_identity_rights_activation_and_global_claim_drift(self):
        for path, value in ((["target_binding", "implementation_NodeId"], "changed"),
                            (["target_binding", "capability_NodeId"], "changed"),
                            (["target_binding", "artifact", "digest"], "sha256:" + "0" * 64),
                            (["target_binding", "activation"], "ACTIVE"),
                            (["rights_and_limits"], "GRANTED"), (["canonical_assignment"], {}),
                            (["source_case_mapping", "supported"], 2937),
                            (["source_case_mapping", "new_supported_indices"], []),
                            (["publication", "commit"], self.witness["commit"])):
            handoff = copy.deepcopy(self.handoff)
            parent = handoff
            for key in path[:-1]:
                parent = parent[key]
            parent[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.validate(handoff=handoff)

    def test_membership_refuses_rebound_commit_tree_blob_path_mode_and_truncation(self):
        mutations = []
        for key, value in (("commit", "0" * 40), ("tree", "0" * 40),
                           ("commit_preimage_utf8", self.witness["commit_preimage_utf8"][:-1]),
                           ("unsigned_content_address_only", False)):
            witness = copy.deepcopy(self.witness)
            witness[key] = value
            mutations.append((key, witness))
        for key, value in (("git_blob", "0" * 40), ("sha256", "0" * 64),
                           ("path", "../elsewhere"), ("mode", "100755"), ("bytes", True)):
            witness = copy.deepcopy(self.witness)
            witness["files"][0][key] = value
            mutations.append(("file:" + key, witness))
        witness = copy.deepcopy(self.witness)
        witness["tree_witness"] = witness["tree_witness"][:-1]
        mutations.append(("missing tree", witness))
        witness = copy.deepcopy(self.witness)
        witness["tree_witness"][0]["entries"][0]["oid"] = "0" * 40
        mutations.append(("changed tree entry", witness))
        for label, witness in mutations:
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.validate(witness=witness)

    def test_reconciliation_is_deterministic_read_only_and_retains_pending_boundaries(self):
        paths = set(self.core.SOURCE_PINS) | set(self.old.PINS) | set(self.helper.PINS)
        before = {path: ((ROOT / path).read_bytes(), (ROOT / path).stat().st_mtime_ns) for path in paths}
        with patch.object(subprocess, "run", side_effect=AssertionError("data comparison cannot execute a child")), \
                patch.object(subprocess, "Popen", side_effect=AssertionError("data comparison cannot spawn a child")):
            report = self.helper.reconcile(ROOT)
            self.assertEqual(report, self.helper.reconcile(ROOT))
        self.assertEqual(report["total"], 2937)
        self.assertEqual(report["supported"], 106)
        self.assertEqual(report["unmapped"], 2831)
        self.assertEqual(report["new_candidate_count"], 7)
        self.assertEqual([row["index"] for row in report["new_candidates"]], NEW)
        self.assertEqual(report["new_manifest_sha256"], NEW_MANIFEST)
        self.assertEqual(report["all106_manifest_sha256"], ALL_MANIFEST)
        self.assertEqual(report["old99"]["count"], 99)
        self.assertIn("NOT_REBOUND", report["old99"]["status"])
        for row in report["new_candidates"]:
            self.assertEqual(row["target"], self.handoff["target_binding"])
            self.assertIn("CustodyUnverified", row["target"]["activation"])
            self.assertIsNone(row["canonical_assignment"])
            self.assertEqual(row["request_presence"], {k: v is not None for k, v in row["request"].items()})
        self.assertTrue(all(row["target"] is None for row in report["unmapped_cases"]))
        self.assertIsNone(report["canonical_assignment"])
        self.assertIsNone(report["accepted_owner"])
        self.assertEqual(report["copying_authorization"], "UNKNOWN")
        self.assertEqual(report["admission"], "PENDING")
        self.assertEqual(report["authority_grants"], [])
        self.assertIs(report["fallback"], False)
        self.assertIs(report["source_adapter_cutover"], False)
        self.assertIs(report["governed_caller_locally_run"], False)
        self.assertEqual(report["original_819_assignments_changed"], 0)
        original = self.core.parse_json((ROOT / self.core.MAPPING).read_bytes())["node_mappings"]
        self.assertEqual(len(original), 819)
        self.assertTrue(all(row["proposed_target"] is None for row in original))
        self.assertEqual(before, {path: ((ROOT / path).read_bytes(), (ROOT / path).stat().st_mtime_ns) for path in paths})

    def test_raw_pins_reject_missing_modified_and_symlink_artifacts(self):
        with self.source_fixture() as (root, _paths):
            self.helper.reconcile(root)
            for relative in self.helper.PINS:
                path = root / relative
                original = path.read_bytes()
                for changed in (b"", original[:-1], original + b"\n"):
                    path.write_bytes(changed)
                    with self.subTest(path=relative, length=len(changed)), self.assertRaises(ValueError):
                        self.helper.reconcile(root)
                path.unlink()
                with self.subTest(path=relative, missing=True), self.assertRaises((ValueError, OSError)):
                    self.helper.reconcile(root)
                path.symlink_to(ROOT / relative)
                with self.subTest(path=relative, symlink=True), self.assertRaises((ValueError, OSError)):
                    self.helper.reconcile(root)
                path.unlink()
                path.write_bytes(original)

    def test_comparison_refuses_inflight_input_byte_or_executable_mode_drift(self):
        with self.source_fixture() as (root, _paths):
            relative = next(iter(self.helper.PINS))
            path = root / relative
            original = path.read_bytes()
            mode = path.stat().st_mode & 0o777
            validate = self.helper.validate
            for kind in ("bytes", "mode"):
                def drift(*args, **kwargs):
                    result = validate(*args, **kwargs)
                    if kind == "bytes":
                        path.write_bytes(original + b"\n")
                    else:
                        path.chmod(mode ^ 0o100)
                    return result

                before_paths = {p.relative_to(root) for p in root.rglob("*")}
                with self.subTest(kind=kind), patch.object(self.helper, "validate", side_effect=drift), \
                        self.assertRaises(ValueError):
                    self.helper.reconcile(root)
                self.assertEqual(before_paths, {p.relative_to(root) for p in root.rglob("*")})
                path.write_bytes(original)
                path.chmod(mode)

    def test_sibling_loader_refuses_nonregular_oversized_symlink_and_changed_helpers(self):
        script = (
            "import runpy,sys\n"
            "module=runpy.run_path(sys.argv[1])\n"
            "try:\n"
            " module['helpers']()\n"
            "except (ValueError,OSError):\n"
            " print('REFUSED')\n"
            " sys.exit(0)\n"
            "sys.exit('unsealed helper was accepted')\n"
        )
        with self.source_fixture() as (root, _paths):
            for name in ("price_handoff.py", "price_subset.py"):
                path = root / QUALIFICATION / name
                original = path.read_bytes()
                for kind in ("fifo", "directory", "oversize", "symlink", "same-size-drift"):
                    path.unlink()
                    if kind == "fifo":
                        os.mkfifo(path)
                    elif kind == "directory":
                        path.mkdir()
                    elif kind == "oversize":
                        path.write_bytes(original + b"\n")
                    elif kind == "symlink":
                        path.symlink_to(ROOT / QUALIFICATION / name)
                    else:
                        path.write_bytes(bytes([original[0] ^ 1]) + original[1:])
                    with self.subTest(helper=name, kind=kind):
                        result = subprocess.run(
                            [sys.executable, "-B", "-c", script, str(root / QUALIFICATION / "price_eur.py")],
                            check=False, capture_output=True, timeout=3,
                            env={"PYTHONDONTWRITEBYTECODE": "1", "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
                        self.assertEqual(result.returncode, 0, result.stderr.decode())
                        self.assertEqual(result.stdout, b"REFUSED\n")
                    if kind == "directory":
                        path.rmdir()
                    else:
                        path.unlink()
                    path.write_bytes(original)

    def test_runner_requires_new_disjoint_physical_output_before_any_child(self):
        with self.source_fixture() as (root, _paths), tempfile.TemporaryDirectory() as temporary:
            io_root = Path(temporary)
            (io_root / "reference").mkdir()
            reference = io_root / "reference/value.rs"
            reference.write_bytes(b"inert\n")
            compiler = io_root / "compiler"
            compiler.write_bytes(b"inert\n")
            compiler.chmod(0o755)
            occupied = io_root / "occupied"
            occupied.mkdir()
            symlink = io_root / "alias"
            symlink.symlink_to(root, target_is_directory=True)
            outputs = [occupied, root / "output", root.parent, reference.parent / "output", compiler,
                       symlink / "output", io_root / "absent/../output"]
            with patch.object(self.helper.subprocess, "Popen", side_effect=AssertionError("guard must precede child")):
                for output in outputs:
                    with self.subTest(output=str(output)), self.assertRaises((ValueError, OSError)):
                        self.helper.run(reference, output, root, compiler)
            self.assertFalse((root / "output").exists())
            self.assertFalse((reference.parent / "output").exists())

    def test_runtime_child_failure_and_launch_error_preserve_stderr_and_isolate_environment(self):
        for failure in ("exit", "launch"):
            with self.subTest(failure=failure), self.refused_runtime_fixture() as (root, reference, compiler, io_root):
                output = io_root / "output"
                commands = []

                def child(command, **kwargs):
                    commands.append((command, kwargs))
                    self.assertEqual(kwargs["cwd"], output)
                    self.assertEqual(set(kwargs["env"]), {"PATH", "HOME", "TMPDIR", "LANG", "LC_ALL", "TZ"})
                    self.assertEqual(kwargs["env"]["HOME"], str(output / "home"))
                    self.assertEqual(kwargs["env"]["TMPDIR"], str(output / "tmp"))
                    self.assertEqual(kwargs["env"]["TZ"], "UTC")
                    if failure == "launch":
                        raise OSError("inert launch refusal")
                    kwargs["stderr"].write(b"inert compiler refusal\n")
                    return SimpleNamespace(returncode=7, poll=lambda: 7)

                with patch.object(self.helper.subprocess, "Popen", side_effect=child), self.assertRaises(ValueError):
                    self.helper.run(reference, output, root, compiler)
                self.assertEqual(len(commands), 1)
                stderr = (output / "command-01.stderr").read_bytes()
                self.assertIn(b"inert", stderr)
                self.assertTrue((output / "command-01.stdout").exists())
                self.assertFalse((output / "qualification.json").exists())
                self.assertFalse((output / "results.json").exists())

    def test_runner_rejects_inflight_compiler_drift_despite_recorded_probe_output(self):
        with self.refused_runtime_fixture() as (root, reference, compiler, io_root):
            output = io_root / "output"
            calls = []
            transcript = []
            for row in self.newresults:
                values = [str(row["index"]), row["status"]]
                if row["status"] == "ERR":
                    values.append(str(row["code"]))
                else:
                    values.extend("-" if row[key] is None else row[key] if key.endswith("bits") else row[key].encode().hex()
                                  for key in self.core.FIELDS)
                transcript.append("\t".join(values) + "\n")

            def child(command, **kwargs):
                calls.append(command)
                if len(calls) == 1:
                    kwargs["stdout"].write(b"inert version for refusal fixture\n")
                elif len(calls) == 2:
                    (output / "probe").write_bytes(b"inert binary for refusal fixture\n")
                    (output / "probe").chmod(0o755)
                elif len(calls) == 3:
                    # These are the retained real outcomes, never invented results.
                    kwargs["stdout"].write("".join(transcript).encode())
                    compiler.write_bytes(compiler.read_bytes() + b"changed\n")
                return SimpleNamespace(returncode=0, poll=lambda: 0)

            with patch.object(self.helper.subprocess, "Popen", side_effect=child), \
                    self.assertRaisesRegex(ValueError, "changed during execution"):
                self.helper.run(reference, output, root, compiler)
            self.assertEqual(len(calls), 3)
            self.assertFalse((output / "qualification.json").exists())
            self.assertFalse((output / "results.json").exists())

    def test_runner_timeout_kills_child_and_retains_bounded_failure_logs(self):
        with self.refused_runtime_fixture() as (root, reference, compiler, io_root):
            output = io_root / "output"
            killed = []
            waited = []
            process = SimpleNamespace(returncode=None, poll=lambda: None,
                                      kill=lambda: killed.append(True), wait=lambda: waited.append(True))
            with patch.object(self.helper.subprocess, "Popen", return_value=process), \
                    patch.object(self.helper.time, "monotonic", side_effect=[0, 61]), \
                    self.assertRaisesRegex(ValueError, "timeout or output limit"):
                self.helper.run(reference, output, root, compiler)
            self.assertEqual(killed, [True])
            self.assertEqual(waited, [True])
            self.assertTrue((output / "command-01.stderr").exists())
            self.assertTrue((output / "command-01.stdout").exists())
            self.assertFalse((output / "qualification.json").exists())


if __name__ == "__main__":
    unittest.main()
