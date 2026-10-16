"""Custody and refusal tests for the bounded price mapping; no governed caller runs."""

import copy
from contextlib import ExitStack, contextmanager
import hashlib
import importlib.util
import json
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


spec = importlib.util.spec_from_file_location("price_subset", Path(__file__).with_name("price_subset.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)
ROOT = Path(__file__).resolve().parents[2]
CUSTODY_PATH = Path("research/commerce/governance/price-text-target-custody-v1.json")
CUSTODY_SHA256 = "b8fc4a80c4efb9def85c298b53172a0c5bf7b2ec817308158cdb2b2d0e110350"
RESULTS_PATH = Path("research/commerce/governance/price-text-pure-probe-v1.json")
RESULTS_SHA256 = "edfd20a74dec11d8ff370d4ab5d859b54de9def6fc75c1caf43b6719cbe6754b"
PROPOSAL_PATH = Path("research/commerce/governance/price-text-subset-proposal-v1.json")
PUBLISHED_HANDOFF_PATH = Path("research/commerce/governance/price-target-published-handoff-v1.json")
PUBLISHED_HANDOFF_SHA256 = "53737eac4519add6701f20a63385db5a82a61b175253779c93cb060e2c542f02"
PUBLISHED_MAP_PATH = Path("research/commerce/governance/price-target-published-case-map-v1.jsonl")
PUBLISHED_MAP_SHA256 = "a320f5e062cabe3c8fa36fdc214a6298d9b84769f1a37da3487ac11ec5116a93"


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def object_id(kind, data):
    return hashlib.sha1(kind.encode() + b" " + str(len(data)).encode() + b"\0" + data).hexdigest()


def tree_id(entries):
    ordered = sorted(entries, key=lambda item: item["path"].encode() + (b"/" if item["type"] == "tree" else b""))
    data = b"".join(
        (item["mode"].lstrip("0") + " " + item["path"]).encode() + b"\0" + bytes.fromhex(item["sha"])
        for item in ordered
    )
    return object_id("tree", data)


def small_tree():
    """A closed recursive Git tree, including file/directory name ordering."""
    nested = [{"path": "value.rs", "mode": "100644", "type": "blob", "sha": object_id("blob", b"value\n"), "size": 6}]
    children = [
        {"path": "a", "mode": "040000", "type": "tree", "sha": tree_id(nested)},
        {"path": "a.txt", "mode": "100644", "type": "blob", "sha": object_id("blob", b"text\n"), "size": 5},
        {"path": "z", "mode": "100755", "type": "blob", "sha": object_id("blob", b"run\n"), "size": 4},
    ]
    return {"sha": tree_id(children), "truncated": False, "tree": children + [dict(nested[0], path="a/value.rs")]}


@contextmanager
def local_reference():
    """Exercise reference I/O using inert local bytes and substituted root pins."""
    data = b"unit-only payload\n"
    blob = object_id("blob", data)
    leaf = {"path": "price_text.rs", "mode": "100644", "type": "blob", "sha": blob, "size": len(data)}
    branch = {"path": "src", "mode": "040000", "type": "tree", "sha": tree_id([leaf])}
    top = {"path": "unit", "mode": "040000", "type": "tree", "sha": tree_id([branch])}
    tree = {"sha": tree_id([top]), "truncated": False,
            "tree": [top, dict(branch, path="unit/src"), dict(leaf, path="unit/src/price_text.rs")]}
    parent = "1" * 40
    preimage = (f"tree {tree['sha']}\nparent {parent}\n"
                "author Unit <unit@example.invalid> 1792108800 +0000\n"
                "committer Unit <unit@example.invalid> 1792108800 +0000\n\nUnit fixture\n").encode()
    commit = {"sha": object_id("commit", preimage), "tree": {"sha": tree["sha"]},
              "parents": [{"sha": parent}], "message": "Unit fixture",
              "author": {"name": "Unit", "email": "unit@example.invalid", "date": "2026-10-16T00:00:00Z"},
              "committer": {"name": "Unit", "email": "unit@example.invalid", "date": "2026-10-16T00:00:00Z"},
              "verification": {"reason": "unsigned", "verified": False, "signature": None, "payload": None}}
    metadata = {"commit": commit["sha"], "tree": tree["sha"],
                "files": [{"path": "unit/src/price_text.rs", "git_blob": blob, "sha256": hashlib.sha256(data).hexdigest(),
                           "bytes": len(data), "mode": "100644"}]}
    with tempfile.TemporaryDirectory() as temporary, ExitStack() as stack:
        root = Path(temporary)
        (root / "unit/src").mkdir(parents=True)
        (root / "unit/src/price_text.rs").write_bytes(data)
        (root / "commit-response.json").write_bytes(canonical(commit))
        (root / "tree-response.json").write_bytes(canonical(tree))
        stack.enter_context(patch.object(runner, "custody", return_value=metadata))
        stack.enter_context(patch.object(runner, "COMMIT", commit["sha"]))
        stack.enter_context(patch.object(runner, "TREE", tree["sha"]))
        stack.enter_context(patch.object(runner, "PARENT", parent))
        stack.enter_context(patch.object(runner, "verify_tree", side_effect=lambda value: runner.verify_git_tree(value)[1]))
        yield root, metadata, tree, commit


def recorded_stdout(results):
    lines = []
    for row in results:
        values = [str(row["index"]), row["status"]]
        if row["status"] == "ERR":
            values.append(str(row["code"]))
        else:
            values.extend("-" if row[key] is None else row[key] if key.endswith("bits") else row[key].encode().hex()
                          for key in runner.FIELDS)
        lines.append("\t".join(values) + "\n")
    return "".join(lines).encode()


class GitCustody(unittest.TestCase):
    def test_sealed_metadata_closes_observed_files_to_commit_without_source_bodies(self):
        data = (ROOT / CUSTODY_PATH).read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), CUSTODY_SHA256)
        metadata = json.loads(data)
        self.assertEqual(object_id("commit", metadata["commit_object_preimage_utf8"].encode()), metadata["commit"])
        witnesses = {row["path"]: row for row in metadata["tree_witness"]}
        self.assertEqual(len(witnesses), 16)
        self.assertEqual(sum(len(row["entries"]) for row in witnesses.values()), 295)
        self.assertEqual(witnesses[""]["oid"], metadata["tree"])
        for path, row in witnesses.items():
            entries = [dict(path=e["name"], sha=e["oid"], mode=e["mode"], type=e["type"]) for e in row["entries"]]
            self.assertEqual(tree_id(entries), row["oid"], path)
            if path:
                parent, _, name = path.rpartition("/")
                self.assertIn({"name": name, "oid": row["oid"], "mode": "040000", "type": "tree"},
                              witnesses[parent]["entries"])
        self.assertEqual(len(metadata["files"]), 9)
        self.assertEqual(len({row["path"] for row in metadata["files"]}), 9)
        for row in metadata["files"]:
            parent, _, name = row["path"].rpartition("/")
            self.assertIn({"name": name, "oid": row["git_blob"], "mode": row["mode"], "type": "blob"},
                          witnesses[parent]["entries"])
            self.assertEqual(set(row), {"path", "git_blob", "mode", "sha256", "bytes", "url"})
        self.assertIsNone(metadata["canonical_assignment"])
        self.assertIsNone(metadata["target_runtime_admission"])
        self.assertIs(metadata["source_adapter_cutover"], False)
        self.assertIn("UNKNOWN", metadata["copying_authorization"])
        self.assertEqual(metadata["pin_order_correction"]["price_child_commit"], metadata["commit"])
        self.assertIn("parent " + metadata["pin_order_correction"]["parent_commit"] + "\n",
                      metadata["commit_object_preimage_utf8"])
        for field, changed in (("canonical_assignment", {}), ("target_runtime_admission", {}),
                               ("source_adapter_cutover", True), ("copying_authorization", "GRANTED")):
            altered = copy.deepcopy(metadata)
            altered[field] = changed
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.custody_to_binding(altered)

    def test_git_objects_and_closed_recursive_tree_have_exact_content_identity(self):
        self.assertEqual(runner.git_oid("blob", b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a")
        tree = small_tree()
        oid, entries = runner.verify_git_tree(tree)
        self.assertEqual(oid, tree["sha"])
        self.assertEqual(set(entries), {"a", "a.txt", "z", "a/value.rs"})
        with self.assertRaises(ValueError):
            runner.verify_tree(tree)

    def test_tree_refuses_truncation_duplicates_paths_types_and_object_drift(self):
        good = small_tree()
        changes = []
        value = copy.deepcopy(good)
        value["truncated"] = True
        changes.append(("truncated", value))
        value = copy.deepcopy(good)
        value["tree"].append(copy.deepcopy(value["tree"][0]))
        changes.append(("duplicate", value))
        for key, bad in (("path", "../outside"), ("path", "/absolute"), ("path", "a//value.rs"),
                         ("mode", "120000"), ("mode", "160000"), ("type", "commit"),
                         ("sha", "0" * 40), ("sha", "bad")):
            value = copy.deepcopy(good)
            value["tree"][-1][key] = bad
            changes.append((f"{key}:{bad}", value))
        value = copy.deepcopy(good)
        value["tree"] = [row for row in value["tree"] if row["path"] != "a"]
        changes.append(("missing ancestor", value))
        value = copy.deepcopy(good)
        value["sha"] = "0" * 40
        changes.append(("root digest", value))
        for label, value in changes:
            with self.subTest(label=label), self.assertRaises(ValueError):
                runner.verify_git_tree(value)

    def test_output_requires_new_physical_disjoint_location(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source, reference = base / "source", base / "reference"
            source.mkdir()
            reference.mkdir()
            for output in (base, source, source / "output", reference, reference / "output"):
                with self.subTest(output=output), self.assertRaises(ValueError):
                    runner.separate_output(output, [source, reference])
            runner.separate_output(base / "new", [source, reference])
            (base / "existing").mkdir()
            with self.assertRaises(ValueError):
                runner.separate_output(base / "existing", [source, reference])
            (base / "link").symlink_to(reference, target_is_directory=True)
            (base / "dangling").symlink_to(base / "missing")
            for value in (base / "link/output", base / "dangling", base / "new/../other"):
                with self.subTest(value=value), self.assertRaises(ValueError):
                    runner.physical_path(value)

    def test_reference_loader_rejects_missing_truncated_rebound_modes_and_symlinks(self):
        with local_reference() as (root, metadata, _tree, _commit):
            path = root / metadata["files"][0]["path"]
            data = path.read_bytes()
            before = path.stat().st_mtime_ns
            self.assertEqual(runner.load_reference(root)["source_code"], data)
            self.assertEqual(path.stat().st_mtime_ns, before)
            for changed in (b"", data[:-1], data + b"changed"):
                path.write_bytes(changed)
                with self.subTest(changed=changed), self.assertRaises(ValueError):
                    runner.load_reference(root)
            path.write_bytes(data)
            path.chmod(0o755)
            with self.assertRaises(ValueError):
                runner.load_reference(root)
            path.chmod(0o644)
            path.unlink()
            with self.assertRaises(ValueError):
                runner.load_reference(root)
            path.symlink_to("missing")
            with self.assertRaises(ValueError):
                runner.load_reference(root)
            path.unlink()
            path.write_bytes(data)
            (root / "extra").write_bytes(b"unexpected")
            with self.assertRaises(ValueError):
                runner.load_reference(root)
            (root / "extra").unlink()
            (root / "linked-directory").symlink_to(root / "unit", target_is_directory=True)
            with self.assertRaises(ValueError):
                runner.load_reference(root)

    def test_reference_commit_and_tree_membership_are_checked_before_compilation(self):
        with local_reference() as (root, metadata, tree, commit):
            commit_path, tree_path = root / "commit-response.json", root / "tree-response.json"
            for field, value in (("sha", "0" * 40), ("tree", {"sha": "0" * 40}),
                                 ("parents", [{"sha": "0" * 40}]), ("message", "rebound")):
                changed = copy.deepcopy(commit)
                changed[field] = value
                commit_path.write_bytes(canonical(changed))
                with self.subTest(commit_field=field), self.assertRaises(ValueError):
                    runner.load_reference(root)
            commit_path.write_bytes(canonical(commit))
            changed = copy.deepcopy(tree)
            changed["truncated"] = True
            tree_path.write_bytes(canonical(changed))
            with self.assertRaises(ValueError):
                runner.load_reference(root)
            tree_path.write_bytes(canonical(tree))
            for field, value in (("git_blob", "0" * 40), ("sha256", "0" * 64), ("bytes", 1), ("mode", "100755")):
                original = metadata["files"][0][field]
                metadata["files"][0][field] = value
                with self.subTest(blob_field=field), self.assertRaises(ValueError):
                    runner.load_reference(root)
                metadata["files"][0][field] = original
            self.assertEqual(runner.load_reference(root)["commit"], commit["sha"])


class PortableMapping(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.corpus, cls.snapshot = runner.load_source(ROOT)
        results_bytes = (ROOT / RESULTS_PATH).read_bytes()
        if hashlib.sha256(results_bytes).hexdigest() != RESULTS_SHA256:
            raise ValueError("recorded pure-probe transcript changed")
        cls.results = runner.parse_json(results_bytes)
        cls.binding = runner.portable_binding()
        cls.proposal = runner.parse_json((ROOT / PROPOSAL_PATH).read_bytes())

    def test_recorded_pure_results_reproduce_exact_portable_pending_proposal(self):
        before = {path: (Path(path).read_bytes(), Path(path).stat().st_mtime_ns) for path in self.snapshot}
        generated = runner.build_proposal(self.corpus, self.results, self.binding)
        metadata = runner.custody()
        runner.inspect(self.proposal, generated)
        self.assertEqual(runner.canonical(generated), runner.canonical(runner.build_proposal(
            copy.deepcopy(self.corpus), copy.deepcopy(self.results), copy.deepcopy(self.binding))))
        self.assertEqual(generated["mapped_count"], 99)
        self.assertEqual(generated["unmapped_count"], 2838)
        self.assertEqual(len(generated["mapped"]), 99)
        self.assertEqual(len(generated["unmapped"]), 2838)
        all_rows = generated["mapped"] + generated["unmapped"]
        self.assertEqual({row["index"] for row in all_rows}, set(range(2937)))
        self.assertEqual(len(all_rows), 2937)
        manifest = "".join(f"{row['index']}\t{row['name']}\n" for row in generated["mapped"])
        self.assertEqual(hashlib.sha256(manifest.encode()).hexdigest(), runner.SUBSET)
        self.assertEqual(generated["qualified_manifest_sha256"], metadata["qualifying_case_manifest_sha256"])
        for row in generated["mapped"]:
            case = self.corpus["cases"][row["index"]]
            self.assertEqual(row["name"], case["name"])
            self.assertEqual(row["corpus_locator"], f"/cases/{row['index']}")
            self.assertEqual(row["request"], {key: case[key] for key in runner.SELECTORS})
            self.assertEqual(row["request_presence"], {key: case[key] is not None for key in runner.SELECTORS})
            self.assertEqual(row["payload_sha256"], hashlib.sha256((case["input"] or "").encode()).hexdigest())
            self.assertEqual(row["status"], "CANDIDATE_PENDING_RIGHTS_AND_ADMISSION")
            self.assertEqual(row["derived"]["amount_float_bits"], None if case["expected"]["amount_float"] is None
                             else struct.pack(">d", case["expected"]["amount_float"]).hex())
            self.assertEqual(row["derived"]["amount_float"], case["expected"]["amount_float"])
            for key in ("amount", "currency", "amount_text"):
                self.assertEqual(row["derived"][key], case["expected"][key])
            self.assertEqual(row["target"]["repository"], metadata["target_repository"])
            self.assertEqual(row["target"]["declared_owner"], metadata["declared_target"]["owner"])
            self.assertEqual(row["target"]["declared_caller"], metadata["declared_target"]["caller"])
            self.assertIsNone(row["canonical_assignment"])
        for row in generated["unmapped"]:
            self.assertIsNone(row["target"])
            self.assertEqual(row["status"], "UNMAPPED")
            self.assertIs(type(row["code"]), int)
            self.assertIn(row["code"], range(1, 17))
        for field in ("canonical_assignment", "accepted_owner"):
            self.assertIsNone(generated[field])
        self.assertEqual(generated["authority_grants"], [])
        self.assertEqual(generated["copying_authorization"], "UNKNOWN")
        self.assertIs(generated["source_adapter_cutover"], False)
        self.assertIs(generated["governed_caller_locally_run"], False)
        self.assertEqual(generated["source"]["original_assignments_changed"], 0)
        self.assertEqual(generated["source"]["implementation_commit"], metadata["source_reference"]["commit"])
        for key in ("handoff_commit", "handoff_path"):
            self.assertEqual(generated["source"][key], metadata["source_reference"][key])
        self.assertEqual(before, {path: (Path(path).read_bytes(), Path(path).stat().st_mtime_ns) for path in before})

    def test_request_wire_preserves_none_empty_unicode_and_selector_presence(self):
        wire = runner.encode_requests(self.corpus).decode("ascii").splitlines()
        self.assertEqual(len(wire), 2937)
        for index, (line, case) in enumerate(zip(wire, self.corpus["cases"])):
            fields = line.split("\t")
            self.assertEqual(len(fields), 5)
            self.assertEqual(fields[0], str(index))
            for key, encoded in zip(runner.SELECTORS, fields[1:]):
                decoded = None if encoded == "-" else bytes.fromhex(encoded).decode("utf-8")
                self.assertEqual(decoded, case[key], (index, key))
        for key in runner.SELECTORS:
            column = runner.SELECTORS.index(key) + 1
            none = next(i for i, case in enumerate(self.corpus["cases"]) if case[key] is None)
            empty = next(i for i, case in enumerate(self.corpus["cases"]) if case[key] == "")
            self.assertEqual(wire[none].split("\t")[column], "-")
            self.assertEqual(wire[empty].split("\t")[column], "")
        changed = copy.deepcopy(self.corpus)
        changed["cases"][0]["currency_hint"] = True
        with self.assertRaises(ValueError):
            runner.encode_requests(changed)

    def test_all_four_derived_fields_include_exact_float_bits_and_null_shape(self):
        success = next(row for row in self.results if row["status"] == "OK" and row["amount"] is not None)
        null = next(row for row in self.results if row["status"] == "OK" and row["amount"] is None)
        for field in runner.FIELDS:
            changed = copy.deepcopy(self.results)
            row = changed[success["index"]]
            row[field] = ("0" * 16 if row[field] != "0" * 16 else "1" * 16) if field.endswith("bits") else "changed"
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.build_proposal(self.corpus, changed, self.binding)
        changed = copy.deepcopy(self.results)
        bits = int(success["amount_float_bits"], 16) ^ 1
        changed[success["index"]]["amount_float_bits"] = f"{bits:016x}"
        with self.assertRaises(ValueError):
            runner.build_proposal(self.corpus, changed, self.binding)
        for field in ("amount", "amount_text", "amount_float_bits"):
            changed = copy.deepcopy(self.results)
            changed[null["index"]][field] = ""
            with self.subTest(null_field=field), self.assertRaises(ValueError):
                runner.build_proposal(self.corpus, changed, self.binding)

    def test_complete_transcript_rejects_reordering_omission_wrong_types_and_forged_success(self):
        success = next(row for row in self.results if row["status"] == "OK")
        unavailable = next(row for row in self.results if row["status"] == "ERR")
        mutations = [self.results[:-1], self.results + [copy.deepcopy(self.results[-1])]]
        changed = copy.deepcopy(self.results)
        changed[0], changed[1] = changed[1], changed[0]
        mutations.append(changed)
        for index, key, value in ((0, "index", True), (0, "index", 1),
                                  (unavailable["index"], "code", True), (unavailable["index"], "code", 0),
                                  (unavailable["index"], "code", 17), (success["index"], "fallback", True),
                                  (unavailable["index"], "status", "UNAVAILABLE")):
            changed = copy.deepcopy(self.results)
            changed[index][key] = value
            mutations.append(changed)
        changed = copy.deepcopy(self.results)
        changed[success["index"]] = {"index": success["index"], "status": "ERR", "code": 8}
        mutations.append(changed)
        changed = copy.deepcopy(self.results)
        case = self.corpus["cases"][unavailable["index"]]
        changed[unavailable["index"]] = {"index": unavailable["index"], "status": "OK",
                                      **{key: case["expected"][key] for key in runner.FIELDS[:-1]},
                                      "amount_float_bits": None if case["expected"]["amount_float"] is None
                                      else struct.pack(">d", case["expected"]["amount_float"]).hex()}
        mutations.append(changed)
        for number, changed in enumerate(mutations):
            with self.subTest(number=number), self.assertRaises(ValueError):
                runner.build_proposal(self.corpus, changed, self.binding)

    def test_source_mutations_and_errors_cannot_be_relabeled_as_original_corpus(self):
        success = next(row["index"] for row in self.results if row["status"] == "OK")
        unavailable = next(row["index"] for row in self.results if row["status"] == "ERR")
        for index, key, value in ((success, "input", "changed"), (unavailable, "currency_hint", ""),
                                  (success, "error", "source failure"), (unavailable, "error", "source failure"),
                                  (success, "name", "changed")):
            changed = copy.deepcopy(self.corpus)
            changed["cases"][index][key] = value
            with self.subTest(index=index, key=key), self.assertRaises(ValueError):
                runner.build_proposal(changed, self.results, self.binding)
        changed = copy.deepcopy(self.corpus)
        changed["cases"][unavailable]["expected"]["amount_text"] = "changed"
        with self.assertRaises(ValueError):
            runner.build_proposal(changed, self.results, self.binding)

    def test_inspection_rejects_self_grants_fallbacks_target_rebinding_and_bool_counts(self):
        mutations = []
        for field, value in (("canonical_assignment", {}), ("accepted_owner", "accepted"),
                             ("authority_grants", ["work"]), ("copying_authorization", "GRANTED"),
                             ("source_adapter_cutover", True), ("governed_caller_locally_run", True),
                             ("mapped_count", True), ("fallback", "source"), ("enrolled", True),
                             ("admitted", True)):
            changed = copy.deepcopy(self.proposal)
            changed[field] = value
            mutations.append((field, changed))
        for field in ("commit", "tree", "metadata_sha256"):
            changed = copy.deepcopy(self.proposal)
            changed["target_custody"][field] = "0" * len(changed["target_custody"][field])
            mutations.append((field, changed))
        changed = copy.deepcopy(self.proposal)
        changed["unmapped"][0]["target"] = copy.deepcopy(changed["mapped"][0]["target"])
        mutations.append(("unmapped target", changed))
        changed = copy.deepcopy(self.proposal)
        changed["mapped"][0]["derived"]["amount_float"] = 42.0
        mutations.append(("derived float", changed))
        for label, changed in mutations:
            with self.subTest(label=label), self.assertRaises(ValueError):
                runner.inspect(changed, self.proposal)
        for field in ("commit", "tree", "metadata_sha256"):
            changed = copy.deepcopy(self.binding)
            changed[field] = "0" * len(changed[field])
            with self.subTest(binding=field), self.assertRaises(ValueError):
                runner.build_proposal(self.corpus, self.results, changed)

    def test_raw_source_pins_reject_changed_missing_and_symlink_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            for relative in runner.SOURCE_PINS:
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, path)
            runner.load_source(source)
            for relative in runner.SOURCE_PINS:
                path = source / relative
                original = path.read_bytes()
                for changed in (original + b"\n", b"", original[:-1]):
                    path.write_bytes(changed)
                    with self.subTest(path=relative, length=len(changed)), self.assertRaises(ValueError):
                        runner.load_source(source)
                path.unlink()
                with self.subTest(missing=relative), self.assertRaises(OSError):
                    runner.load_source(source)
                path.symlink_to(ROOT / relative)
                with self.subTest(symlink=relative), self.assertRaises(ValueError):
                    runner.load_source(source)
                path.unlink()
                path.write_bytes(original)
            runner.load_source(source)

    def test_runner_isolates_child_environment_and_refuses_inflight_input_drift(self):
        transcript = recorded_stdout(self.results)
        self.assertEqual(runner.parse_results(transcript), self.results)
        for drift in (None, "source", "compiler", "compile-failure"):
            with self.subTest(drift=drift), tempfile.TemporaryDirectory() as temporary:
                base = Path(temporary)
                source, reference, output = base / "source", base / "reference", base / "output"
                reference.mkdir()
                for relative in runner.SOURCE_PINS:
                    path = source / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(ROOT / relative, path)
                compiler = base / "rustc"
                compiler.write_bytes(b"inert test compiler")
                binding = {**self.binding, "source_code": b"inert test module", "snapshot": {}}

                def execute(args, **options):
                    self.assertEqual(options["cwd"], output)
                    self.assertIs(options["check"], False)
                    self.assertEqual(options["timeout"], 60)
                    for forbidden in ("GH_TOKEN", "GIT_DIR", "RUSTC_WRAPPER", "CARGO_ENCODED_RUSTFLAGS"):
                        self.assertNotIn(forbidden, options["env"])
                    self.assertEqual(options["env"]["HOME"], str(output / "home"))
                    self.assertEqual(options["env"]["TMPDIR"], str(output / "tmp"))
                    if "--version" in args:
                        if drift == "source":
                            path = source / runner.CORPUS
                            path.write_bytes(path.read_bytes() + b"\n")
                        elif drift == "compiler":
                            compiler.write_bytes(b"changed test compiler")
                        stdout = b"inert version\n"
                    elif "--edition=2021" in args:
                        if drift == "compile-failure":
                            return SimpleNamespace(stdout=b"", stderr=b"synthetic compiler failure", returncode=2)
                        (output / "probe").write_bytes(b"inert test binary")
                        stdout = b""
                    else:
                        self.assertEqual(args, [str(output / "probe")])
                        self.assertEqual(options["input"], runner.encode_requests(self.corpus))
                        stdout = transcript
                    return SimpleNamespace(stdout=stdout, stderr=b"", returncode=0)

                with patch.dict(runner.os.environ, {"GH_TOKEN": "must-not-enter-child", "GIT_DIR": "unexpected",
                                                   "RUSTC_WRAPPER": "unexpected", "CARGO_ENCODED_RUSTFLAGS": "unexpected"}), \
                        patch.object(runner, "load_reference", return_value=binding), \
                        patch.object(runner.subprocess, "run", side_effect=execute) as commands:
                    if drift == "compile-failure":
                        with self.assertRaisesRegex(ValueError, "synthetic compiler failure"):
                            runner.run(reference, output, source, compiler)
                        self.assertEqual((output / "command-02.stderr").read_bytes(), b"synthetic compiler failure")
                        self.assertFalse((output / "qualification.json").exists())
                        self.assertFalse((output / "proposal.json").exists())
                    elif drift:
                        with self.assertRaisesRegex(ValueError, "changed during execution"):
                            runner.run(reference, output, source, compiler)
                        self.assertFalse((output / "qualification.json").exists())
                        self.assertFalse((output / "proposal.json").exists())
                    else:
                        receipt = runner.run(reference, output, source, compiler)
                        self.assertEqual(receipt["supported"], 99)
                        self.assertEqual(receipt["unmapped"], 2838)
                        self.assertIs(receipt["governed_caller_locally_run"], False)
                        self.assertIsNone(receipt["canonical_assignment"])
                        self.assertEqual(receipt["copying_authorization"], "UNKNOWN")
                        self.assertEqual(runner.parse_json((output / "results.json").read_bytes()), self.results)
                        self.assertEqual(runner.parse_json((output / "proposal.json").read_bytes()), self.proposal)
                    self.assertEqual(commands.call_count, 2 if drift == "compile-failure" else 3)


class TranscriptDecoder(unittest.TestCase):
    def test_decoder_preserves_optional_fields_and_rejects_ambiguous_or_nonfinite_json(self):
        rows = runner.parse_results(b"0\tOK\t-\t\t30\t3ff0000000000000\n1\tERR\t8\n")
        self.assertEqual(rows, [{"index": 0, "status": "OK", "amount": None, "currency": "", "amount_text": "0",
                                 "amount_float_bits": "3ff0000000000000"},
                                {"index": 1, "status": "ERR", "code": 8}])
        for value in (b'{"field":1,"field":2}', b'{"a":1,"\\u0061":2}',
                      b'{"nested":{"a":1,"a":2}}', b'{"value":NaN}', b'{"value":Infinity}'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                runner.parse_json(value)
        for value in (b"0\n", b"\n", b"x\tERR\t8\n", b"00\tERR\t8\n", b"0\tOTHER\t8\n",
                      b"0\tOK\t-\t-\tzz\t-\n"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                runner.parse_results(value)


class PublishedPortableHandoff(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        helper_spec = importlib.util.spec_from_file_location("price_handoff", Path(__file__).with_name("price_handoff.py"))
        cls.helper = importlib.util.module_from_spec(helper_spec)
        helper_spec.loader.exec_module(cls.helper)
        handoff_bytes = (ROOT / PUBLISHED_HANDOFF_PATH).read_bytes()
        mapping_bytes = (ROOT / PUBLISHED_MAP_PATH).read_bytes()
        if hashlib.sha256(handoff_bytes).hexdigest() != PUBLISHED_HANDOFF_SHA256:
            raise ValueError("published handoff bytes changed")
        if hashlib.sha256(mapping_bytes).hexdigest() != PUBLISHED_MAP_SHA256:
            raise ValueError("published case mapping bytes changed")
        cls.handoff = runner.parse_json(handoff_bytes)
        cls.rows = [runner.parse_json(line) for line in mapping_bytes.splitlines()]
        cls.corpus, _ = runner.load_source(ROOT)
        cls.results = runner.parse_json((ROOT / RESULTS_PATH).read_bytes())
        cls.proposal = runner.parse_json((ROOT / PROPOSAL_PATH).read_bytes())
        mapping = cls.handoff["source_case_mapping"]
        cls.request_domain = re.search(r"str\(([^)]+)\)", mapping["request_digest"]).group(1)
        cls.projection_domain = re.search(r"str\(([^)]+)\)", mapping["projection_digest"]).group(1)

    @contextmanager
    def source_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = set(runner.SOURCE_PINS) | set(self.helper.PINS) | {
                ".ecdev/qualification/price_subset.py", ".ecdev/qualification/price_handoff.py"}
            for relative in paths:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / relative, path)
            with patch.object(self.helper, "__file__", str(root / ".ecdev/qualification/price_handoff.py")):
                yield root, paths

    def test_portable_encoding_keeps_optional_utf8_and_exact_float_bits(self):
        self.assertEqual(self.helper.encode_string("é"), b"\x02\x00\x00\x00\xc3\xa9")
        self.assertEqual(self.helper.optional_string(None), b"\x00")
        self.assertEqual(self.helper.optional_string(""), b"\x01\x00\x00\x00\x00")
        for value in (True, 1, []):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.helper.optional_string(value)
        case = {key: None for key in runner.SELECTORS}
        for key in runner.SELECTORS:
            changed = {**case, key: ""}
            self.assertNotEqual(self.helper.request_digest(case, "unit"), self.helper.request_digest(changed, "unit"))
        fields = {"amount": None, "currency": None, "amount_text": None, "amount_float": -0.0}
        expected = b"\x04\x00\x00\x00unit" + b"\x00" * 3 + b"\x01" + struct.pack("<d", -0.0)
        self.assertEqual(self.helper.projection_digest(fields, "unit"), hashlib.sha256(expected).hexdigest())
        native = {key: fields[key] for key in runner.FIELDS[:-1]}
        native["amount_float_bits"] = struct.pack(">d", -0.0).hex()
        self.assertEqual(self.helper.projection_digest(native, "unit"), self.helper.projection_digest(fields, "unit"))
        self.assertNotEqual(self.helper.projection_digest(fields, "unit"),
                            self.helper.projection_digest({**fields, "amount_float": 0.0}, "unit"))
        for value in (float("nan"), float("inf"), True):
            with self.subTest(float_value=value), self.assertRaises(ValueError):
                self.helper.projection_digest({**fields, "amount_float": value}, "unit")
        for changed in ({**native, "amount_float_bits": "3ff0000000000000", "amount_float": True},
                        {**native, "amount_float": 1.0}, {**native, "amount_float_bits": "7ff0000000000000"},
                        {**native, "amount_float_bits": "bad"}):
            with self.subTest(native=changed), self.assertRaises(ValueError):
                self.helper.projection_digest(changed, "unit")

    def test_all_published_rows_match_original_requests_source_projections_and_actual_results(self):
        self.helper.validate_rows(self.corpus, self.results, self.rows, self.handoff)
        self.assertEqual(len(self.rows), 2937)
        supported = []
        for index, (case, row, result) in enumerate(zip(self.corpus["cases"], self.rows, self.results)):
            self.assertEqual(row["index"], index)
            self.assertEqual(row["request_sha256"], self.helper.request_digest(case, self.request_domain), index)
            self.assertEqual(row["source_projection_sha256"],
                             self.helper.projection_digest(case["expected"], self.projection_domain), index)
            if result["status"] == "OK":
                self.assertEqual(row["native"], "SUPPORTED", index)
                self.assertEqual(row["result_sha256"], row["source_projection_sha256"], index)
                self.assertEqual(row["result_sha256"], self.helper.projection_digest(result, self.projection_domain), index)
                supported.append(index)
            else:
                self.assertEqual(row["native"], "UNMAPPED", index)
                self.assertIs(type(row["unavailable"]), int)
                self.assertEqual(row["unavailable"], result["code"], index)
        self.assertEqual(len(supported), 99)
        self.assertEqual(len(self.rows) - len(supported), 2838)
        self.assertEqual(supported, self.handoff["source_case_mapping"]["supported_indices"])
        self.assertEqual(supported, [row["index"] for row in self.proposal["mapped"]])
        self.assertEqual(self.handoff["target_publication"]["commit"], self.proposal["target_custody"]["commit"])
        self.assertEqual(self.handoff["target_publication"]["tree"], self.proposal["target_custody"]["tree"])

    def test_portable_rows_refuse_hash_index_shape_status_and_typed_code_drift(self):
        supported = next(row["index"] for row in self.rows if row["native"] == "SUPPORTED")
        unavailable = next(row["index"] for row in self.rows if row["native"] == "UNMAPPED")
        mutations = [("omitted", self.rows[:-1]), ("extra", self.rows + [copy.deepcopy(self.rows[-1])])]
        changed = copy.deepcopy(self.rows)
        changed[0], changed[1] = changed[1], changed[0]
        mutations.append(("reordered", changed))
        for index, key, value in ((0, "index", True), (0, "index", 1), (0, "request_sha256", "0" * 64),
                                  (0, "source_projection_sha256", "0" * 64),
                                  (supported, "result_sha256", "0" * 64), (supported, "native", "UNMAPPED"),
                                  (unavailable, "native", "SUPPORTED"), (unavailable, "unavailable", True),
                                  (unavailable, "unavailable", 17),
                                  (unavailable, "unavailable", 1 if self.rows[unavailable]["unavailable"] != 1 else 2),
                                  (supported, "admitted", True), (unavailable, "fallback", "source")):
            changed = copy.deepcopy(self.rows)
            changed[index][key] = value
            mutations.append((f"{index}:{key}:{value}", changed))
        for label, changed in mutations:
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.helper.validate_rows(self.corpus, self.results, changed, self.handoff)

    def test_reconciliation_is_read_only_and_retains_identity_candidates_without_activation(self):
        paths = set(runner.SOURCE_PINS) | set(self.helper.PINS)
        before = {p: ((ROOT / p).read_bytes(), (ROOT / p).stat().st_mtime_ns) for p in paths}
        with patch.object(runner.subprocess, "run", side_effect=AssertionError("data comparison cannot execute a child")):
            report = self.helper.reconcile(ROOT)
            again = self.helper.reconcile(ROOT)
        self.assertEqual(report, again)
        self.assertEqual(report["total"], 2937)
        self.assertEqual(report["supported"], 99)
        self.assertEqual(report["unmapped"], 2838)
        self.assertEqual(report["published_candidate"], self.handoff["target_binding"])
        self.assertEqual(report["source_ids"], self.proposal["source"]["source_ids"])
        self.assertEqual(report["source_ids"]["source_capability"],
                         self.handoff["request_result_admission"]["source"]["native_source_capability"])
        self.assertEqual(report["published_candidate"]["artifact"]["digest"],
                         "sha256:" + self.proposal["mapped"][0]["target"]["implementation_key"].rsplit(":", 1)[1])
        self.assertEqual(report["target_activation"], self.handoff["target_binding"]["activation"])
        self.assertIn("CustodyUnverified", report["target_activation"])
        self.assertIsNone(report["canonical_assignment"])
        self.assertIsNone(report["accepted_owner"])
        self.assertEqual(report["copying_authorization"], "UNKNOWN")
        self.assertEqual(report["registry_binding"], "PENDING")
        self.assertIs(report["source_adapter_cutover"], False)
        self.assertIs(report["governed_caller_locally_run"], False)
        self.assertEqual(report["original_819_assignments_changed"], 0)
        original = runner.parse_json((ROOT / runner.MAPPING).read_bytes())["node_mappings"]
        self.assertEqual(len(original), 819)
        self.assertTrue(all(row["proposed_target"] is None for row in original))
        self.assertEqual(before, {p: ((ROOT / p).read_bytes(), (ROOT / p).stat().st_mtime_ns) for p in before})

    def test_handoff_identity_rights_claims_results_and_original_proposal_are_sealed(self):
        for path, value in ((["target_binding", "capability_NodeId"], "changed"),
                            (["target_binding", "implementation_NodeId"], "changed"),
                            (["target_binding", "artifact", "id"], "changed"),
                            (["target_binding", "activation"], "ACTIVE"),
                            (["rights_and_adoption"], "GRANTED"),
                            (["canonical_assignment"], {}),
                            (["source_case_mapping", "supported_indices"], [])):
            changed = copy.deepcopy(self.handoff)
            parent = changed
            for key in path[:-1]:
                parent = parent[key]
            parent[path[-1]] = value
            with self.subTest(path=path), self.assertRaises(ValueError):
                self.helper.validate_rows(self.corpus, self.results, self.rows, changed)
        changed_results = copy.deepcopy(self.results)
        unavailable = next(row["index"] for row in changed_results if row["status"] == "ERR")
        changed_results[unavailable]["code"] = 1 if changed_results[unavailable]["code"] != 1 else 2
        with self.assertRaises(ValueError):
            self.helper.validate_rows(self.corpus, changed_results, self.rows, self.handoff)
        changed_proposal = copy.deepcopy(self.proposal)
        changed_proposal["canonical_assignment"] = {}
        with self.assertRaises(ValueError):
            self.helper.validate(self.corpus, self.results, changed_proposal, self.handoff, self.rows)

    def test_comparison_refuses_raw_drift_symlinks_and_inflight_change_without_writes(self):
        with self.source_fixture() as (root, _paths):
            self.helper.reconcile(root)
            pinned = set(runner.SOURCE_PINS) | set(self.helper.PINS) | {".ecdev/qualification/price_subset.py"}
            for relative in pinned:
                path = root / relative
                original = path.read_bytes()
                for changed in (b"", original + b"\n", original[:-1]):
                    path.write_bytes(changed)
                    with self.subTest(path=relative, length=len(changed)), self.assertRaises(ValueError):
                        self.helper.reconcile(root)
                path.write_bytes(original)
            path = root / self.helper.HANDOFF_PATH
            original = path.read_bytes()
            path.unlink()
            path.symlink_to(ROOT / self.helper.HANDOFF_PATH)
            with self.assertRaises(ValueError):
                self.helper.reconcile(root)
            path.unlink()
            path.write_bytes(original)
            validate = self.helper.validate

            def change_after_validation(*args):
                checked = validate(*args)
                path.write_bytes(original + b"\n")
                return checked

            existing_paths = {p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}
            with patch.object(self.helper, "validate", side_effect=change_after_validation), \
                    self.assertRaisesRegex(ValueError, "changed during execution"):
                self.helper.reconcile(root)
            self.assertEqual({p.relative_to(root).as_posix() for p in root.rglob("*") if p.is_file()}, existing_paths)

    def test_sibling_core_loader_refuses_fifo_directory_and_oversize_without_blocking(self):
        script = (
            "import runpy,sys\n"
            "module=runpy.run_path(sys.argv[1])\n"
            "try:\n"
            " module['helpers']()\n"
            "except (ValueError,OSError):\n"
            " print('REFUSED')\n"
            " sys.exit(0)\n"
            "sys.exit('malformed core was accepted')\n"
        )
        with self.source_fixture() as (root, _paths):
            path = root / ".ecdev/qualification/price_subset.py"
            original = path.read_bytes()
            for kind in ("fifo", "directory", "oversize"):
                path.unlink()
                if kind == "fifo":
                    runner.os.mkfifo(path)
                elif kind == "directory":
                    path.mkdir()
                else:
                    path.write_bytes(original + b"\n")
                with self.subTest(kind=kind):
                    result = subprocess.run(
                        [sys.executable, "-B", "-c", script, str(root / ".ecdev/qualification/price_handoff.py")],
                        check=False, capture_output=True, timeout=3,
                        env={"PYTHONDONTWRITEBYTECODE": "1", "PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
                    self.assertEqual(result.returncode, 0, result.stderr.decode())
                    self.assertEqual(result.stdout, b"REFUSED\n")
                if kind == "directory":
                    path.rmdir()
                else:
                    path.unlink()
                path.write_bytes(original)


if __name__ == "__main__":
    unittest.main()
