"""Exact retained cohort bytes, immutable source inputs and closed caller rebinding."""

import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).absolute().parents[2]
MODULE_PATH = ROOT / ".ecdev/qualification/price_manifest.py"
PIN = "300591b5db1f9b156ecf5282525e5d056c79986140cffff20fcd784b2e825792"
MANIFEST_PIN = "451f0bb1a2f3acd3879ddc2945c14e74447ec6ac70dfd5ebf374d086c8f353dc"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True) + "\n").encode()


class RetainedPriceManifest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("price_manifest", MODULE_PATH)
        cls.helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.helper)
        cls.corpus_path = ROOT / cls.helper.CORPUS_PATH
        cls.sidecar_path = ROOT / cls.helper.SIDECAR_PATH
        cls.corpus = cls.corpus_path.read_bytes()
        cls.sidecar = cls.sidecar_path.read_bytes()
        cls.value = json.loads(cls.sidecar)
        cls.artifact = (ROOT / cls.helper.MANIFEST_PATH).read_bytes()

    def build(self, corpus=None, sidecar=None, pin=PIN):
        return self.helper.build_manifest(self.corpus if corpus is None else corpus,
                                          self.sidecar if sidecar is None else sidecar, pin)

    def inspect(self, manifest):
        return self.helper.inspect_manifest(self.corpus, self.sidecar, PIN, manifest)

    def cli(self, *args):
        return subprocess.run([sys.executable, "-B", str(MODULE_PATH), *args], cwd=ROOT,
                              env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"},
                              check=False, capture_output=True, timeout=10)

    def test_actual_106_manifest_matches_published_pin_and_exact_original_names(self):
        actual = self.build()
        cases = json.loads(self.corpus)["cases"]
        expected = "".join(f"{i}\t{cases[i]['name']}\n"
                           for i in self.value["coverage"]["qualified_case_indices"]).encode()
        self.assertEqual(actual, expected)
        self.assertEqual(actual, self.artifact)
        self.assertEqual(sha256(actual), MANIFEST_PIN)
        self.assertEqual((len(actual), actual.count(b"\n")), (4051, 106))
        checked = self.inspect(actual)
        self.assertEqual(checked["manifest_sha256"], MANIFEST_PIN)
        self.assertEqual((checked["total"], checked["supported"], checked["unmapped"]), (2937, 106, 2831))
        self.assertEqual(checked["scope"], "INDEX_NAME_METADATA_ONLY; NO_CANONICAL_ASSIGNMENT_OR_ADMISSION")

    def test_exact_seven_added_and_all_original_99_preserved(self):
        additions = [117, 212, 409, 855, 1188, 1193, 1849]
        rows = self.artifact.splitlines(keepends=True)
        by_index = {int(row.split(b"\t", 1)[0]): row for row in rows}
        self.assertEqual([row["index"] for row in self.value["new_candidates"]], additions)
        self.assertTrue(set(additions).issubset(by_index))
        new = b"".join(row for i, row in by_index.items() if i in additions)
        old = b"".join(row for i, row in by_index.items() if i not in additions)
        self.assertEqual((new.count(b"\n"), old.count(b"\n")), (7, 99))
        self.assertEqual(sha256(new), "5beaf6ab248dde7a3c42e6ef1b177c64e8c3af481561e5dd2d76aa2d893d342e")
        self.assertEqual(sha256(old), "d5794485dd05bcb51e79fcc5af2a0ea6a1a915800a829ee0211cd3df985df29f")

    def test_all_2937_original_cases_and_raw_corpus_sidecar_case_map_bytes_unchanged(self):
        paths = [self.corpus_path, self.sidecar_path,
                 ROOT / "research/commerce/governance/price-eur-target-case-map-v1.jsonl"]
        before = {path: path.read_bytes() for path in paths}
        cases = json.loads(self.corpus)["cases"]
        self.assertEqual(len(cases), 2937)

        def snapshots(values):
            # Preserve all fields, null/empty distinctions and advisory IEEE bits,
            # including valid exponent spellings that decode to infinity.
            return [(json.dumps(case, sort_keys=True),
                     None if case.get("expected") is None or case["expected"]["amount_float"] is None
                     else struct.pack(">d", case["expected"]["amount_float"])) for case in values]

        original_cases = snapshots(cases)
        self.inspect(self.build())
        self.assertEqual(snapshots(json.loads(self.corpus)["cases"]), original_cases)
        self.assertEqual({path: path.read_bytes() for path in paths}, before)
        self.assertEqual((self.corpus, self.sidecar), (before[self.corpus_path], before[self.sidecar_path]))

    def test_caller_must_supply_the_exact_fixed_sidecar_pin(self):
        for pin in (None, True, b"300591b5", "0" * 64, "sha256:" + PIN, PIN.upper()):
            with self.subTest(pin=pin), self.assertRaises(ValueError):
                self.build(pin=pin)

    def test_sidecar_cohort_edits_and_authority_self_grants_cannot_be_caller_rebound(self):
        edits = [
            lambda d: d["coverage"]["qualified_case_indices"].__setitem__(0, True),
            lambda d: d["coverage"]["qualified_case_indices"].__setitem__(0, 46),
            lambda d: d["coverage"]["qualified_case_indices"].reverse(),
            lambda d: d["coverage"]["qualified_case_indices"].__setitem__(0, 117),
            lambda d: d["new_candidates"][0].__setitem__("name", "edited"),
            lambda d: d.__setitem__("authority", "granted"),
            lambda d: d.__setitem__("accepted_owner", "self-appointed"),
            lambda d: d.__setitem__("copying_authorization", "AUTHORIZED"),
            lambda d: d.__setitem__("canonical_assignment", "assigned"),
            lambda d: d.__setitem__("admission", "ADMITTED"),
            lambda d: d["authority_grants"].append("execution"),
        ]
        for edit in edits:
            value = copy.deepcopy(self.value)
            edit(value)
            raw = encode(value)
            for pin in (PIN, sha256(raw)):
                with self.subTest(edit=edit, pin=pin), self.assertRaises(ValueError):
                    self.build(sidecar=raw, pin=pin)

    def test_source_proposal_or_fresh_capture_cannot_replace_retained_bounded_sidecar(self):
        for relative in ("source-eight-mapping-db92bf27-v1.json", "source-eight-mapping-499a3074-v1.json",
                         "source-eight-capture-499a3074-v1.json"):
            raw = (ROOT / "research/commerce/governance" / relative).read_bytes()
            for pin in (PIN, sha256(raw)):
                with self.subTest(path=relative, pin=pin), self.assertRaises(ValueError):
                    self.build(sidecar=raw, pin=pin)

    def test_any_corpus_byte_or_case_change_rejected_including_unselected_fields(self):
        corpus = json.loads(self.corpus)
        variants = [self.corpus + b"\n"]
        for index, field, value in ((45, "name", "edited"), (0, "input", "edited"),
                                     (45, "expected", {"amount_float": 0}), (0, "name", True)):
            edited = copy.deepcopy(corpus)
            edited["cases"][index][field] = value
            variants.append(encode(edited))
        for raw in variants:
            with self.subTest(digest=sha256(raw)), self.assertRaises(ValueError):
                self.helper.build_manifest(raw, self.sidecar, PIN)

    def test_manifest_framing_index_name_order_and_duplicate_mutations_rejected(self):
        rows = self.artifact.splitlines(keepends=True)
        variants = [self.artifact[:-1], self.artifact.replace(b"\n", b"\r\n"),
                    b"".join(reversed(rows)), b"".join([rows[0], *rows[:-1]]),
                    self.artifact.replace(b"45\t", b"true\t", 1),
                    self.artifact.replace(b"45\t", b"46\t", 1),
                    self.artifact.replace(rows[0], b"45\tedited\n", 1),
                    self.artifact + b"\n", bytearray(self.artifact), None]
        for index, raw in enumerate(variants):
            with self.subTest(index=index), self.assertRaises(ValueError):
                self.inspect(raw)

    def test_bounded_bytes_required(self):
        for raw in (None, bytearray(self.corpus), b"x" * (self.helper.MAX_INPUT + 1)):
            with self.subTest(kind=type(raw)), self.assertRaises(ValueError):
                self.helper.build_manifest(raw, self.sidecar, PIN)
        with self.assertRaises(ValueError):
            self.build(sidecar=b'{"coverage":null,"coverage":true}')

    def test_cli_requires_independent_pin_and_emits_exact_artifact_or_inspection(self):
        missing = self.cli()
        self.assertEqual(missing.returncode, 2)
        self.assertEqual(missing.stdout, b"")
        built = self.cli("--expected-sidecar-sha256", PIN)
        self.assertEqual((built.returncode, built.stderr, built.stdout), (0, b"", self.artifact))
        inspected = self.cli("--expected-sidecar-sha256", PIN, "--inspect-manifest",
                             str(ROOT / self.helper.MANIFEST_PATH))
        self.assertEqual(inspected.returncode, 0, inspected.stderr)
        self.assertEqual(json.loads(inspected.stdout), self.inspect(self.artifact))
        wrong = self.cli("--expected-sidecar-sha256", "0" * 64)
        self.assertEqual((wrong.returncode, wrong.stdout), (2, b""))

    def test_cli_rejects_edited_inspection_and_nonregular_or_linked_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            edited = directory / "edited.tsv"
            edited.write_bytes(self.artifact[:-1])
            link = directory / "linked.json"
            link.symlink_to(self.corpus_path)
            fifo = directory / "input.fifo"
            os.mkfifo(fifo)
            for option, path in (("--inspect-manifest", edited), ("--corpus", link),
                                 ("--corpus", directory), ("--corpus", fifo)):
                result = self.cli("--expected-sidecar-sha256", PIN, option, str(path))
                self.assertEqual((result.returncode, result.stdout), (2, b""), result.stderr)


if __name__ == "__main__":
    unittest.main()
