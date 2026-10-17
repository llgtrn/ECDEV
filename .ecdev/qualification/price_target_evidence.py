#!/usr/bin/env python3
"""Reconcile published temporary-world price receipts without accepting production gates."""

import argparse
import base64
import binascii
import copy
import hashlib
import math
from pathlib import Path
import re
import struct
import sys

import price_candidate as candidate
import price_caller_readiness as caller
import price_cohort as cohort
import price_custody as custody


ROOT = Path(__file__).absolute().parents[2]
TARGET_PATH = "research/commerce/governance/price-target-test-only-outcomes-053e8a8f-v1.json"
TARGET_SHA256 = "f9fa2abeb82d39cb707f6249197e6ec115639953de1db472b50827d82d4696a4"
TARGET_BLOB = "7ffe9500526f30a4927b2090a6f799ceb3d946cd"
TARGET_BYTES = 474084
TARGET_PUBLICATION = "053e8a8f8e85a137d80e8c9fe70ff5dde3c7ebb3"
SOURCE_COMMIT = "36265383228654f725ed6307d8a69f8d569f36d0"
SOURCE_TREE = "b0a275c165b621e6399384b6494a161d7c6d108f"
WITNESS_PATH = "research/commerce/governance/price-selected-source-custody-db92bf27-v1.json"
WITNESS_SHA256 = "e6f58d934cc1b55d949dc925f83510f5361432f64a3b5620424da3a01bc8d2b0"
PREREQUISITES_PATH = "research/commerce/governance/price-source-parity-cutover-prerequisites-v1.json"
PREREQUISITES_SHA256 = "6b2a1a93dfe79a503999650826b0d93c6442b022f77380a6738216a515974560"
U64_MAX = 18446744073709551615
OBSERVED = [117, 212, 319, 365, 855, 1849, 2170, 2914]
SELECTORS = ("input_is_none", "currency_hint", "decimal_separator", "digit_group_separator")
MARKER = "ECDEV362_TEST_ONLY_CASE "
PRIMARY_READBACK = (
    ("lineara/src/work_receipt.rs", "085c28b78aa3a9d4c20162f36b193e04a0d2542b", "38b1bc57e195156fae968afc47fe41aaaff853556c0d3452a100639b5900a8f6", 32923),
    ("phenora/codara/src/price_text_result.rs", "948861d16dda541b85ad87e73bc45da76110ce1d", "2ee394bbca54d26c032bee49b49b3feda3e3fd7db2452354218520a23343802f", 10477),
    ("metabion/src/work_run.rs", "7d6ae1f65d8a6149eda1b0e88562edac924f5d95", "e0b2bc96eeadc579362e49a67e61e7ac178f5e45dd72a1bbec31cee7fe2156d8", 19845),
    ("physiology/host/src/allocation.rs", "c81e8843e36b7dcf0c7eeefe659d846767c55ca7", "70ca38c35c59fd5e4daaaf9b4621d91caeb67b03163fe1ec952fd2e6ada61ded", 100101),
    ("phenora/codara/src/run.rs", "f1cc32b2cd687929d8190984adc6c5f79e89c25c", "fa176c74e1b04015083522b958ec8476737a4262094a41e574abeeb2414c386e", 97242),
    ("phenora/codara/tests/e2e_price_text.rs", "7d13a298b72c13fc43106f125582fcfbaa425e71", "17b863f039ecfc1ba14ba651347556962933db125a9eb4b5c9dc17709e978bb3", 63868),
    ("assays/gate/fixtures/ecdev-selected-bindings/selected-four-36265383.json", "8b03b6e8bedfd530617b629574db9369cb638f5f", "06bf05e79b2d36b404f677dc47c4b39f6be18abe287f9348225edc86e4c231cf", 20316),
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def same(actual, expected, message):
    custody.same(actual, expected, message)


def uint(value):
    require(type(value) is int and 0 <= value <= U64_MAX, "exact unsigned 64-bit integer required")
    return value


def packed_text(text):
    raw = text.encode()
    return struct.pack("<I", len(raw)) + raw


class Reader:
    def __init__(self, raw, limit):
        require(type(raw) is bytes and len(raw) <= limit, "bounded receipt bytes required")
        self.raw, self.offset = raw, 0

    def take(self, size):
        require(0 <= size <= len(self.raw) - self.offset, "truncated receipt")
        result = self.raw[self.offset:self.offset + size]
        self.offset += size
        return result

    def number(self, kind):
        return struct.unpack(kind, self.take(struct.calcsize(kind)))[0]

    def u8(self):
        return self.number("<B")

    def vector(self, maximum=1024):
        size = self.number("<I")
        require(size <= maximum, "receipt vector exceeds bound")
        return self.take(size)

    def text(self, required=False):
        text = self.vector().decode("utf-8")
        if required:
            require(bool(text) and not any(ord(c) < 32 or ord(c) == 127 for c in text),
                    "nonempty receipt identity without control characters required")
        return text

    def digest(self):
        raw = self.vector(32)
        require(len(raw) == 32, "32-byte receipt digest required")
        return raw.hex()

    def optional(self, read):
        tag = self.u8()
        require(tag in (0, 1), "invalid optional receipt tag")
        return None if tag == 0 else read()

    def finish(self):
        require(self.offset == len(self.raw), "trailing receipt bytes")


def wire_parameters(report):
    """Authority spelling is sealed wire data, never a source control import or runtime call."""
    prefix, _ = report["primary_ordered106_receipts"][0]["temporary_world_canonical_receipt"].split(" ", 1)
    require(prefix.endswith(".holding-work1") and prefix.count(".") == 1, "unsupported receipt prefix")
    authority = prefix.partition(".")[0]
    return {"prefix": prefix, "authority": authority,
            "checksum_domain": (prefix.removesuffix("1") + ".v1\0").encode(),
            "request_domain": (authority + ".request.v1\0").encode()}


def decode_result(raw):
    reader = Reader(raw, 512)
    require(reader.number("<H") == 1, "unsupported result version")
    kind = reader.text(required=True)
    result = {"kind": kind, "input_sha256": reader.digest(), "ir_sha256": reader.digest(),
              "selectors_sha256": reader.digest()}
    outcome = reader.u8()
    require(outcome in (0, 1), "unsupported price outcome tag")
    if outcome == 0:
        code = reader.u8()
        require(1 <= code <= 16, "invalid known-unavailable result code")
        result.update(outcome="KNOWN_UNAVAILABLE", unavailable_code=code, fields=None, amount_float_bits=None)
    else:
        fields = {key: reader.optional(reader.text) for key in ("amount", "currency", "amount_text")}
        bits = reader.optional(lambda: reader.number("<Q"))
        floating = None if bits is None else struct.unpack("<d", struct.pack("<Q", bits))[0]
        require(floating is None or math.isfinite(floating), "nonfinite advisory result float")
        fields["amount_float"] = floating
        require(fields["currency"] in (None, "$", "USD", "EUR"), "unsupported raw currency token")
        if fields["amount"] is None:
            require(fields["amount_text"] is None and bits is None, "inconsistent nullable source result")
        else:
            require(type(fields["amount_text"]) is str and bits is not None, "incomplete numeric source result")
            require(len(fields["amount"]) <= 32 and len(fields["amount_text"]) <= 32,
                    "oversize numeric source projection")
            require(re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", fields["amount"]) is not None,
                    "invalid derived decimal text")
            require(caller.float_bits(float(fields["amount"])) == f"{bits:016x}", "decimal/float bits inconsistent")
        result.update(outcome="SUPPORTED", unavailable_code=None, fields=fields,
                      amount_float_bits=None if bits is None else f"{bits:016x}")
    reader.finish()
    return result


def decode_receipt(receipt, parameters):
    require(type(receipt) is str, "receipt text required")
    prefix, separator, encoded = receipt.partition(" ")
    require(separator and prefix == parameters["prefix"], "receipt prefix differs")
    require(len(encoded) <= 8192 and re.fullmatch(r"(?:[0-9a-f]{2})+", encoded) is not None,
            "bounded canonical lowercase receipt hex required")
    raw = bytes.fromhex(encoded)
    require(len(raw) >= 34, "receipt body missing")
    body, checksum = raw[:-32], raw[-32:]
    require(hashlib.sha256(parameters["checksum_domain"] + body).digest() == checksum,
            "receipt checksum differs")
    reader = Reader(body, 4096)
    require(reader.number("<H") == 1, "unsupported receipt version")
    decoded = {"work_run": reader.text(required=True), "effect": reader.number("<Q"),
               "generation": reader.number("<Q"), "lease": reader.text(required=True),
               "provider": reader.text(required=True), "request_sha256": reader.digest(),
               "requester": reader.text(required=True), "resource": reader.text(required=True),
               "holding": reader.text(required=True)}
    decoded["consumed"] = reader.optional(lambda: reader.number("<q"))
    tag = reader.u8()
    require(tag in (0, 1, 2), "invalid completion tag")
    decoded["completed"] = (None, False, True)[tag]
    result = reader.optional(reader.vector)
    require(result is not None, "actual price receipt result required")
    decoded["result"] = decode_result(result)
    decoded["result_hex"] = result.hex()
    reader.finish()
    decoded.update(receipt_sha256=cohort.digest(receipt.encode()), wire_bytes=len(raw),
                   checksum_sha256=checksum.hex())
    return decoded


def request_digest(parameters, parts):
    framed = b"".join(struct.pack("<Q", len(part)) + part for part in parts)
    return hashlib.sha256(parameters["request_domain"] + framed).digest()


def selector_facts(case):
    facts = {key: case[key] for key in SELECTORS[1:] if case[key] is not None}
    if case["input"] is None:
        facts["input_is_none"] = "true"
    require(all(type(value) is str for value in facts.values()), "original selector text Option required")
    return facts


def selector_digest(facts):
    # Published Writer grammar: domain text followed by each number-Option and text-Option.
    raw = packed_text("codara.price-text.selectors.v1")
    for name in SELECTORS:
        raw += b"\0"  # Numeric facts are absent for these original source requests.
        value = facts.get(name)
        raw += b"\0" if value is None else b"\1" + packed_text(value)
    return cohort.digest(raw)


def markers(stdout):
    rows = []
    for line in stdout.splitlines():
        if not line.startswith(MARKER):
            continue
        fields, separator, receipt = line[len(MARKER):].partition(" receipt=")
        require(separator, "marker receipt missing")
        fields = fields.split(" ")
        names = ("route", "index", "input_sha256", "ir", "request")
        require(len(fields) == len(names), "marker fields differ")
        values = {}
        for name, field in zip(names, fields):
            require(field.startswith(name + "="), "marker field order differs")
            values[name] = field[len(name) + 1:]
        require(re.fullmatch(r"0|[1-9][0-9]*", values["index"]) is not None, "noncanonical marker index")
        rows.append({"route": values["route"], "index": uint(int(values["index"])),
                     "input_sha256": values["input_sha256"], "ir_sha256": values["ir"],
                     "request_sha256": values["request"], "temporary_world_canonical_receipt": receipt})
    return rows


def check_commands(report):
    commands = report["command_receipts"]
    expected = {"test": (0, (5, 0, 0)), "attempt01-test": (101, (3, 2, 0)),
                "default-price-text-file": (0, (14, 0, 6)), "scoped-clippy": (101, None),
                "scoped-clippy-no-deps": (101, None), "single-lint-filtered-diagnostic": (0, None),
                "scoped-fmt": (0, None)}
    require(set(commands) == set(expected), "all seven original command records required")
    for name, (exit_status, test_counts) in expected.items():
        row = commands[name]
        require(type(row["exit_status"]) is int and row["exit_status"] == exit_status,
                "original command outcome changed")
        for stream in ("stdout", "stderr"):
            require(cohort.digest(row[stream + "_utf8"].encode()) == row[stream + "_sha256"],
                    "raw command stream digest differs")
        if test_counts is not None:
            found = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;", row["stdout_utf8"])
            same([tuple(map(int, counts)) for counts in found], [test_counts], "test count summary differs")
    for name in ("scoped-clippy", "scoped-clippy-no-deps"):
        require("chunks_exact_to_as_chunks" in commands[name]["stderr_utf8"], "inherited strict lint blocker missing")
        require("-D" in commands[name]["argv"] and "warnings" in commands[name]["argv"], "strict lint arguments differ")
    require("-A" in commands["single-lint-filtered-diagnostic"]["argv"], "filtered diagnostic must remain filtered")
    try:
        raw_stdout = base64.b64decode(report["raw_success_stdout_base64"], validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError("invalid exact raw stdout base64") from error
    require(cohort.digest(raw_stdout) == report["raw_success_stdout_sha256"], "exact raw stdout digest differs")
    require(raw_stdout == commands["test"]["stdout_utf8"].encode(), "raw stdout differs from actual successful command")
    require(commands["test"]["source_pin"] == SOURCE_COMMIT, "test command source revision differs")
    same(commands["test"]["test_source_sha256"], report["test_source_sha256"], "test source binding differs")
    same(commands["test"]["fixture_sha256"], report["metadata_fixture_sha256"], "test metadata binding differs")
    return raw_stdout


def reconcile_rows(report, contract, corpus):
    parameters = wire_parameters(report)
    raw_stdout = check_commands(report)
    observed_markers = markers(raw_stdout.decode())
    primary, controls = report["primary_ordered106_receipts"], report["supplemental_control_receipts"]
    require(len(primary) == 106 and len(controls) == 3 and len(observed_markers) == 109,
            "106 primary cases and three actual controls required")
    web = contract["selected_web_indices"]
    api = [row["index"] for row in contract["API_only_exclusions"]]
    same([(row["route"], uint(row["index"])) for row in primary],
         [("Web", index) for index in web] + [("API_ONLY", index) for index in api],
         "exact 99 Web then seven API-only receipt order required")
    same([(row["route"], uint(row["index"])) for row in controls],
         [("CONTROL_BUDGET", 45), ("CONTROL_HOLDING", 45), ("KNOWN_UNAVAILABLE", U64_MAX)],
         "exact supplemental control identities required")
    same([row for row in observed_markers if row["route"] in ("Web", "API_ONLY")], primary,
         "ordered primary receipts differ from actual stdout")
    same([row for row in observed_markers if row["route"] not in ("Web", "API_ONLY")], controls,
         "ordered control receipts differ from actual stdout")
    counts = {route: sum(row["route"] == route for row in observed_markers)
              for route in ("Web", "API_ONLY", "CONTROL_BUDGET", "CONTROL_HOLDING", "KNOWN_UNAVAILABLE")}
    same(report["marker_counts"], counts, "marker counts differ")
    target = contract["existing_target_reference"]
    kernel = target["implementation_key"].split("@", 1)[0]
    kind = "codara.commerce.price.parse-text"
    summaries = []
    for row in primary + controls:
        index = uint(row["index"])
        boundary = row["route"] == "KNOWN_UNAVAILABLE"
        case = {"input": "$12.99", "currency_hint": "USD", "decimal_separator": None,
                "digit_group_separator": None} if boundary else corpus["cases"][index]
        payload = b"" if case["input"] is None else case["input"].encode()
        facts = selector_facts(case)
        decoded = decode_receipt(row["temporary_world_canonical_receipt"], parameters)
        result = decoded["result"]
        require(result["kind"] == kind, "unsupported price result kind")
        same(result["input_sha256"], cohort.digest(payload), "receipt payload digest differs from original source")
        same(result["input_sha256"], row["input_sha256"], "receipt/marker input digest join differs")
        same(result["ir_sha256"], row["ir_sha256"], "receipt/marker full IR digest join differs")
        same(result["selectors_sha256"], selector_digest(facts), "original selector Option commitment differs")
        native = request_digest(parameters, [kind.encode() + b".v1", bytes.fromhex(result["ir_sha256"]),
                                            target["implementation_NodeId"].encode(), target["capability_NodeId"].encode(),
                                            kernel.encode(), hashlib.sha256(payload).digest()])
        same(native.hex(), row["request_sha256"], "native request/input/full IR/implementation join differs")
        lease_key = {"CONTROL_BUDGET": "ecdev362-first", "CONTROL_HOLDING": "ecdev362-correct-holding",
                     "KNOWN_UNAVAILABLE": "ecdev362-boundary"}.get(row["route"], f"ecdev362-{index}")
        same(decoded["lease"], lease_key + ".0.lease", "original case/run lease differs")
        same((decoded["effect"], decoded["generation"], decoded["requester"], decoded["resource"],
              decoded["holding"], decoded["provider"], decoded["consumed"]),
             (1, 0, "agent", "cpu-1", "test-ecdev362", "kernel:" + kernel, 1),
             "temporary-world receipt context differs")
        base = request_digest(parameters, [b"lease-work", decoded["requester"].encode(), decoded["resource"].encode(),
                                           decoded["lease"].encode(), decoded["provider"].encode()])
        bound_native = request_digest(parameters, [base, b"native-input", native])
        expected_outer = request_digest(parameters, [(parameters["authority"] + ".lease-work.holding.v1").encode(),
                                                     bound_native, decoded["holding"].encode()])
        same(decoded["request_sha256"], expected_outer.hex(), "complete outer request/Holding join differs")
        fence = request_digest(parameters, [(parameters["prefix"].removesuffix("1") + ".lease-fence.v1").encode(),
                                            decoded["resource"].encode(), decoded["lease"].encode()])
        same(decoded["work_run"], fence.hex(), "Holding lease fence/EffectRef join differs")
        if boundary:
            require(result["outcome"] == "KNOWN_UNAVAILABLE" and result["unavailable_code"] == 8
                    and decoded["completed"] is False, "boundary must be known UnsupportedSyntax code 8, not unknown")
        else:
            require(result["outcome"] == "SUPPORTED" and decoded["completed"] is True, "qualified case is not known successful")
            expected = case["expected"]
            for field in ("amount", "currency", "amount_text"):
                same(result["fields"][field], expected[field], "original source result field differs")
            same(result["amount_float_bits"], caller.float_bits(expected["amount_float"]), "original source float bits differ")
        summaries.append({"route": row["route"], "index": index, "source_request": copy.deepcopy(case) if boundary else caller.request(case),
                          "source_selector_facts": facts, "payload_utf8_hex": payload.hex(),
                          "native_request_sha256": native.hex(), "decoded_receipt": decoded,
                          "temporary_world_canonical_receipt": row["temporary_world_canonical_receipt"],
                          "scope": "TEMPORARY_TEST_WORLD_ONLY", "production_authority_granted": False})
    return summaries


def build_assessment(root=ROOT, *, expected_target_sha256):
    root = candidate.physical(root)
    raw = candidate.regular(root / TARGET_PATH)
    cohort.require_pin(raw, expected_target_sha256)
    cohort.require_pin(raw, TARGET_SHA256)
    require(len(raw) == TARGET_BYTES and custody.blob_id(raw) == TARGET_BLOB, "published raw target report identity differs")
    report = candidate.strict_json(raw)  # Python int preserves the exact u64MAX control index.
    require(type(report["supplemental_control_receipts"][-1]["index"]) is int, "u64 sentinel must remain an integer")
    same((report["source_commit"], report["source_tree"]), (SOURCE_COMMIT, SOURCE_TREE), "fixed source revision differs")
    require(report["source_files_rechecked_after_tests"] is True and report["original_test_prefix_exact"] is True,
            "original source/test readback preservation missing")
    entries = custody.tree_entries(root, SOURCE_COMMIT, SOURCE_TREE)
    files = report["source_files"]
    require(len(files) == 14 and len({row["source_path"] for row in files}) == 14, "fourteen exact source inputs required")
    source_bytes = {}
    for row in files:
        member = custody.committed_file(root, entries, row["source_path"], include_hex=False, worktree=True)
        same(member, {"path": row["source_path"], "mode": "100644", "blob": row["git_blob"],
                      "sha256": row["sha256"], "bytes": row["bytes"]}, "source report membership differs")
        source_bytes[row["source_path"]] = candidate.regular(root / row["source_path"])
    witness_raw, contract_raw = source_bytes[WITNESS_PATH], source_bytes[PREREQUISITES_PATH]
    cohort.require_pin(witness_raw, WITNESS_SHA256)
    cohort.require_pin(contract_raw, PREREQUISITES_SHA256)
    custody.inspect_witness(witness_raw, WITNESS_SHA256, root, expected_report_sha256=custody.REPORT_SHA256,
                            expected_readiness_sha256=custody.READINESS_SHA256)
    witness, contract = candidate.strict_json(witness_raw), candidate.strict_json(contract_raw)
    same(contract, witness["parity_cutover_prerequisites"], "source prerequisite contract differs from witness")
    same(report["original_six_obligation_evidence"], {row["key"]: None for row in contract["obligations"]},
         "original null obligations changed")
    same(report["original_source_status"], contract["status"], "original source status changed")
    source_report = candidate.strict_json(source_bytes[custody.REPORT_PATH])
    observed = [row["index"] for row in source_report["cases"] if row["source_observed"]["product_price_status"] == "OBSERVED"]
    same(observed, OBSERVED, "eight original observed source cases differ")
    same(report["source_observed_indices"], OBSERVED, "target report misstates observed source indices")
    same(report["source_observation_counts"], {"OBSERVED": 8, "UNKNOWN": 91}, "source observation counts differ")
    same(report["boundary_scope"]["original20_names"], [row["name"] for row in contract["source_boundary_records"]],
         "original twenty source boundary names differ")
    require(report["boundary_scope"]["executed_boundary_count"] == 1, "only one actual target boundary was executed")
    same(report["metadata_fixture_sha256"], PRIMARY_READBACK[-1][2], "published target metadata digest differs")
    same(report["test_source_sha256"], PRIMARY_READBACK[-2][2], "published test source digest differs")
    corpus = candidate.strict_json(source_bytes[custody.manifest.CORPUS_PATH])
    decoded = reconcile_rows(report, contract, corpus)
    provenance = report["target_runtime_provenance"]
    same(provenance, {"base_head": "cc99cd28db0f4e914727d0095f8e9e1899b8e79e",
                      "working_tree_not_pristine": True,
                      "description": "Base HEAD plus owned appended tests and concurrent separately reviewed ordinary changes; executable hash identifies actual tested binary.",
                      "binary_sha256": "d6470a700d2bffcb166db4e48856031535dd81a55d647161cddcf68f54b0af76",
                      "cargo_lock_sha256": "a9798d8a6d376e0f1def4fdeab8e7061b4b6766ce55ddc131165cba69b3552b9"},
         "actual test runtime provenance changed")
    obligations = {row["key"]: {"production_status": "OPEN", "original_evidence": None,
                                "test_context_subevidence": None} for row in contract["obligations"]}
    obligations["target_execution"]["test_context_subevidence"] = {
        "kind": "TEMPORARY_WORLD_109_RECEIPTS_RECONCILED", "Web_cases": 99, "API_only_cases": 7,
        "controls": 3, "production_execution_established": False}
    obligations["authority"]["test_context_subevidence"] = {
        "kind": "TEMPORARY_TEST_AUTHENTICATION_HOLDING_GRANT_AND_BUDGET_REFUSALS",
        "provenance": "Published successful test command and source assertions; no operational grant"}
    obligations["unknown_effect"]["test_context_subevidence"] = {
        "kind": "TEMPORARY_NATIVE_WORK_RUN_STORE_PENDING_STATE_TEST",
        "provenance": "Published test asserts reopen preserves unknown/no reexecution/no log change; no real crash or production effect claim"}
    assessment = {"format": "ecdev-price-target-test-evidence-assessment-v1", "status": "TEST_EVIDENCE_RECONCILED_ONLY",
                  "target_report": {"path": TARGET_PATH, "sha256": TARGET_SHA256, "git_blob": TARGET_BLOB,
                                    "bytes": TARGET_BYTES, "publication_commit": TARGET_PUBLICATION},
                  "source_revision": {"commit": SOURCE_COMMIT, "tree": SOURCE_TREE, "source_files": copy.deepcopy(files),
                                      "fourteen_committed_and_worktree_members_rechecked": True},
                  "selected_source": {"witness_sha256": WITNESS_SHA256, "prerequisites_sha256": PREREQUISITES_SHA256,
                                      "bindings": [row["binding"] for row in witness["selected_nodes"]],
                                      "fixed_current_selected_material_rechecked": True},
                  "counts": copy.deepcopy(contract["counts"]), "primary_receipts": decoded[:106], "control_receipts": decoded[106:],
                  "wire_frames_reconciled": len(decoded), "source_observation_counts": {"OBSERVED": 8, "UNKNOWN": 91},
                  "source_observed_indices": OBSERVED, "boundary_execution": {"executed": 1, "name": "$12.99",
                                                                            "known_unavailable_code": 8,
                                                                            "original_source_boundaries": 20,
                                                                            "remaining_boundaries_not_claimed_executed": 19,
                                                                            "control_index": U64_MAX},
                  "raw_success_stdout_sha256": report["raw_success_stdout_sha256"],
                  "command_validation": {"targeted_tests": {"passed": 5, "failed": 0, "ignored": 0},
                                         "normal_tests": {"passed": 14, "failed": 0, "ignored": 6},
                                         "initial_attempt": {"passed": 3, "failed": 2, "retained": True},
                                         "strict_clippy": "BLOCKED_INHERITED_LINT", "no_deps_strict_clippy": "BLOCKED_INHERITED_LINT",
                                         "single_lint_filtered_diagnostic": "EXIT_ZERO_FILTERED_DIAGNOSTIC_ONLY", "fmt": "PASS",
                                         "original_interpretation": copy.deepcopy(report["validation_interpretation"]),
                                         "commands": {key: {field: row[field] for field in ("argv", "exit_status", "stdout_sha256", "stderr_sha256")}
                                                      for key, row in report["command_receipts"].items()}},
                  "runtime_provenance": copy.deepcopy(provenance), "exact_published_revision_binary_attestation": False,
                  "primary_codec_references": [{"commit": TARGET_PUBLICATION, "path": path, "blob": blob,
                                                "sha256": digest, "bytes": size} for path, blob, digest, size in PRIMARY_READBACK],
                  "original_six_obligation_evidence": copy.deepcopy(report["original_six_obligation_evidence"]),
                  "original_source_status": copy.deepcopy(report["original_source_status"]),
                  "obligation_assessment": obligations,
                  "production_status": {"status": "CUTOVER_BLOCKED", "authority_granted": False, "current_rights": None,
                                        "canonical_enrollment": "BLOCKED", "source_adapter_cutover": False,
                                        "exclusive_custody": None, "production_qualification_established": False},
                  "limits": ["Temporary test worlds and content checksums establish no production authority or authorship",
                             "Full IR digest is joined across actual markers, result and native/outer request; capsule/full IR bytes were not exported for independent reconstruction",
                             "Actual tested binary/lock and dirty base are recorded separately from the later publication revision",
                             "All six original source obligation fields stay null and all production gates remain open",
                             "No target runtime call, reenrollment, copying authorization or effect dispatch is performed by this inspector"]}
    require(candidate.regular(root / TARGET_PATH) == raw, "target outcome input changed during reconciliation")
    for path, before in source_bytes.items():
        require(candidate.regular(root / path) == before, "source input changed during reconciliation")
    return assessment


def inspect_assessment(raw, expected_assessment_sha256, root=ROOT, *, expected_target_sha256):
    cohort.require_pin(raw, expected_assessment_sha256)
    expected = build_assessment(root, expected_target_sha256=expected_target_sha256)
    same(candidate.strict_json(raw), expected, "test-only evidence assessment, exact integers, receipts or authority limits changed")
    return {"format": "ecdev-price-target-test-evidence-inspection-v1", "status": "TEST_EVIDENCE_RECONCILED_ONLY",
            "assessment_sha256": cohort.digest(raw), "target_report_sha256": TARGET_SHA256,
            "primary_cases": 106, "Web_cases": 99, "API_only_cases": 7, "control_receipts": 3,
            "wire_frames_reconciled": 109, "source_observed": 8, "source_unknown": 91,
            "production_status": "CUTOVER_BLOCKED", "authority_granted": False, "source_adapter_cutover": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--expect-target-sha256", required=True)
    parser.add_argument("--inspect-assessment", type=Path)
    parser.add_argument("--expect-assessment-sha256")
    args = parser.parse_args()
    if bool(args.inspect_assessment) != bool(args.expect_assessment_sha256):
        parser.error("assessment input and independent SHA-256 must be supplied together")
    try:
        kwargs = {"expected_target_sha256": args.expect_target_sha256}
        result = (inspect_assessment(candidate.regular(args.inspect_assessment), args.expect_assessment_sha256,
                                     args.source_root, **kwargs) if args.inspect_assessment
                  else build_assessment(args.source_root, **kwargs))
        sys.stdout.buffer.write(cohort.encode(result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError, UnicodeError) as error:
        print(f"target test evidence refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
