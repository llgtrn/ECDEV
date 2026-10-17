#!/usr/bin/env python3
"""Compare pinned published CLI evidence with the existing offline source reader.

Only recorded packet verification is supported. No target binary, package code,
Git command or network operation is executed. A recorded source-only receipt
does not close a production obligation or establish operational authority.
"""

import argparse
import base64
import binascii
from pathlib import Path
import sys

import price_custody as custody
import price_source_package as package


MAX_PROFILE_BYTES = 128 * 1024
MAX_ARTIFACT_BYTES = 1024 * 1024
MAX_RECEIPT_BYTES = 16 * 1024
FORMAT = "ecdev-selected-package-interoperability-v1"
RESULT_STATUS = "SOURCE_ONLY_SELECTED_MEMBERSHIP"
READ_ONLY = {
    "status": RESULT_STATUS,
    "claim": package.CLAIM,
    "accepted_obligations": 0,
    "enrollment_credit": 0,
    "authority": False,
    "runtime_admission": "NO_GRANT",
    "source_execution": "NOT_EXECUTED",
    "whole_capture": False,
    "production_obligations": "ALL_SIX_OPEN",
    "source_adapter_cutover": False,
    "source_retirement": False,
    "target_exclusive_custody": None,
    "Host_admission": None,
    "current_production_rights": None,
}


def same(actual, expected, label):
    """Canonical JSON comparison also distinguishes booleans from integers."""
    package.require(package.encode(actual) == package.encode(expected), label + " differs")


def bounded_json(raw, maximum, label):
    package.require(type(raw) is bytes and len(raw) <= maximum, label + " byte budget exceeded")
    return package.strict_json(raw)


def artifact(raw, expected):
    package.keys(expected, ("path", "sha256", "bytes", "git_blob", "published_path"), "artifact pin")
    package.safe_path(expected["path"])
    package.safe_path(expected["published_path"])
    package.pin(expected["sha256"], 64, "artifact SHA-256")
    package.pin(expected["git_blob"], 40, "artifact Git blob")
    package.count(expected["bytes"], MAX_ARTIFACT_BYTES, "artifact bytes")
    package.require(type(raw) is bytes and len(raw) == expected["bytes"], "artifact length mismatch")
    package.require(package.digest(raw) == expected["sha256"], "artifact SHA-256 mismatch")
    package.require(package.git_digest("blob", raw) == expected["git_blob"], "artifact Git blob mismatch")


def required_argv(argv, pins):
    package.require(type(argv) is list and all(type(arg) is str for arg in argv), "CLI argv invalid")
    package.require(len(argv) in (14, 16) and argv[1:3] == ["migrate", "inspect-selected"]
                    and argv[-1] == "--json", "selected CLI command required")
    expected = {"--sha256": pins["package_sha256"], "--commit": pins["source_commit"],
                "--tree": pins["source_tree"], "--rights-sha256": pins["rights_sha256"]}
    options = argv[3:-1:2]
    package.require(set(options) == set(expected) | {"--file"}
                    | ({"--root"} if len(argv) == 16 else set()), "CLI options differ")
    package.require(all(value and not value.startswith("--") for value in argv[4:-1:2]),
                    "CLI option value missing")
    for flag, value in expected.items():
        package.require(argv.count(flag) == 1, "independent source flag must occur exactly once")
        position = argv.index(flag)
        package.require(position + 1 < len(argv) and argv[position + 1] == value,
                        "independent CLI source pin mismatch")
    package.require(argv.count("--json") == 1, "JSON CLI output required exactly once")


def decode_log(raw):
    package.require(type(raw) is bytes and len(raw) <= 24 * 1024,
                    "encoded receipt byte budget exceeded")
    # Accept canonical single-line or standard 76-column LF-wrapped base64.
    # No other whitespace or alternate encoding is accepted.
    encoded = raw.replace(b"\n", b"")
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError("invalid receipt base64") from exc
    canonical = base64.b64encode(decoded)
    package.require(len(decoded) <= MAX_RECEIPT_BYTES and canonical == encoded
                    and raw in (canonical, canonical + b"\n", base64.encodebytes(decoded)),
                    "noncanonical or oversized receipt base64")
    return decoded


def inspect_recorded_package(source_raw, expected_package_sha256, *, profile_raw,
                             expected_profile_sha256, artifacts, expected_central_commit,
                             expected_central_tree, expected_source_commit, expected_source_tree,
                             expected_rights_sha256):
    """Verify a reviewed immutable packet and compose the unchanged source reader.

    All seven pins must arrive independently of these input bytes. Artifact
    mappings use exactly the three role names from the reviewed profile.
    """
    for value, size, label in ((expected_profile_sha256, 64, "profile SHA-256"),
                               (expected_central_commit, 40, "central commit"),
                               (expected_central_tree, 40, "central tree"),
                               (expected_package_sha256, 64, "package SHA-256"),
                               (expected_source_commit, 40, "source commit"),
                               (expected_source_tree, 40, "source tree"),
                               (expected_rights_sha256, 64, "rights SHA-256")):
        package.pin(value, size, label)
    package.require(type(profile_raw) is bytes and len(profile_raw) <= MAX_PROFILE_BYTES,
                    "profile byte budget exceeded")
    package.require(package.digest(profile_raw) == expected_profile_sha256, "independent profile pin mismatch")
    profile = bounded_json(profile_raw, MAX_PROFILE_BYTES, "profile")
    package.keys(profile, ("format", "scope", "central_repository", "central_commit", "central_tree",
                           "central_binding", "binary", "package_pins", "artifacts", "expected_argv",
                           "expected_central_result", "full_scope_contract", "qualification_scope"), "profile")
    package.require(profile["format"] == "ecdev-selected-central-contract-v1"
                    and profile["scope"] == "RECORDED_PUBLISHED_CLI_SOURCE_ONLY_SELECTED_MEMBERSHIP",
                    "profile scope changed")
    package.require(profile["central_commit"] == expected_central_commit
                    and profile["central_tree"] == expected_central_tree, "independent central pins mismatch")
    pins = {"package_sha256": expected_package_sha256, "source_commit": expected_source_commit,
            "source_tree": expected_source_tree, "rights_sha256": expected_rights_sha256}
    same(profile["package_pins"], pins, "independent source profile pins")
    package.keys(profile["artifacts"], ("interoperability", "cli_capture", "cli_log"), "profile artifacts")
    package.keys(artifacts, profile["artifacts"], "recorded artifacts")
    for role, expected in profile["artifacts"].items():
        artifact(artifacts[role], expected)
    package.require(len({row["path"] for row in profile["artifacts"].values()}) == 3,
                    "artifact paths must be distinct")
    required_argv(profile["expected_argv"], pins)
    package.require(len(profile["expected_argv"]) == 16, "recorded CLI root required")
    expected_result = {**READ_ONLY, **pins, "material_files": 83, "evidence_files": 3, "tree_objects": 19}
    same(profile["expected_central_result"], expected_result, "closed source-only result contract")
    interop = bounded_json(artifacts["interoperability"], MAX_ARTIFACT_BYTES, "interoperability")
    package.keys(interop, ("scope", "binary", "binding", "cli_argv", "result_contract",
                           "exact_result_receipt", "rust_api", "source_counterpart",
                           "full_scope_unchanged", "caller_requirements"), "interoperability")
    package.require(interop["scope"] == "Read-only selected SOURCE_ONLY byte intake; not full-source qualification or production enrollment",
                    "interoperability scope changed")
    package.keys(profile["binary"], ("platform", "sha256", "bytes"), "profile binary")
    for value in (profile["central_repository"], profile["qualification_scope"],
                  profile["binary"]["platform"], interop["rust_api"], interop["source_counterpart"],
                  interop["full_scope_unchanged"]):
        package.require(type(value) is str and 0 < len(value) <= 4096, "bounded metadata text required")
    package.pin(profile["binary"]["sha256"], 64, "binary SHA-256")
    package.count(profile["binary"]["bytes"], 1024 * 1024 * 1024, "binary bytes")
    package.keys(interop["binary"], (*profile["binary"], "saved_readonly_path", "portable_alternative"),
                 "interoperability binary")
    for key in ("saved_readonly_path", "portable_alternative"):
        package.require(type(interop["binary"][key]) is str and 0 < len(interop["binary"][key]) <= 4096,
                        "bounded binary metadata text required")
    package.require(type(interop["caller_requirements"]) is list
                    and 0 < len(interop["caller_requirements"]) <= 16
                    and all(type(value) is str and 0 < len(value) <= 4096
                            for value in interop["caller_requirements"]), "caller requirements invalid")
    same({key: interop["binary"][key] for key in profile["binary"]}, profile["binary"], "binary metadata")
    package.keys(profile["central_binding"], ("schema", "engine", "protocol_version", "compatibility_epoch"),
                 "central binding")
    for key in ("schema", "engine"):
        binding = profile["central_binding"][key]
        package.require(type(binding) is str and binding.startswith("sha256:"), "binding digest required")
        package.pin(binding[7:], 64, "binding digest")
    for key in ("protocol_version", "compatibility_epoch"):
        package.count(profile["central_binding"][key], 1000000, "binding version")
    same(interop["binding"], profile["central_binding"], "central binding")
    required_argv(interop["cli_argv"], pins)
    contract_keys = ("status", "material_files", "evidence_files", "tree_objects", "accepted_obligations",
                     "enrollment_credit", "authority", "runtime_admission", "whole_capture",
                     "production_obligations", "source_adapter_cutover", "Host_admission",
                     "target_exclusive_custody", "current_production_rights")
    same(interop["result_contract"], {"success_exit": 0, "refusal_exit": 2,
                                     **{key: expected_result[key] for key in contract_keys}},
         "interoperability result contract")
    full = profile["full_scope_contract"]
    package.keys(full, ("path", "sha256", "portable_format", "files", "nodes", "unchanged"), "full scope metadata")
    package.safe_path(full["path"])
    package.pin(full["sha256"], 64, "full scope SHA-256")
    same({key: full[key] for key in ("portable_format", "files", "nodes", "unchanged")},
         {"portable_format": "ecdev-source-capture-v1", "files": 734, "nodes": 828, "unchanged": True},
         "full scope metadata")
    capture = bounded_json(artifacts["cli_capture"], MAX_ARTIFACT_BYTES, "CLI capture")
    package.keys(capture, ("name", "command", "started", "seconds", "exit", "log", "encoding", "raw_name",
                           "bytes", "sha256"), "CLI capture")
    same(capture["command"], profile["expected_argv"], "recorded CLI argv")
    required_argv(capture["command"], pins)
    package.require(type(capture["exit"]) is int and capture["exit"] == 0
                    and capture["encoding"] == "base64", "recorded CLI did not succeed")
    package.require(type(capture["seconds"]) in (int, float) and 0 <= capture["seconds"] <= 86400,
                    "recorded CLI duration invalid")
    for key in ("name", "started", "raw_name"):
        package.require(type(capture[key]) is str and 0 < len(capture[key]) <= 1024,
                        "CLI capture metadata invalid")
    package.safe_path(capture["log"])
    package.require(profile["artifacts"]["cli_log"]["published_path"].endswith("/" + capture["log"]),
                    "recorded CLI log path mismatch")
    same(interop["exact_result_receipt"], profile["artifacts"]["cli_capture"]["path"]
         + " + " + capture["log"], "recorded receipt locator")
    receipt_raw = decode_log(artifacts["cli_log"])
    package.count(capture["bytes"], MAX_RECEIPT_BYTES, "receipt bytes")
    package.pin(capture["sha256"], 64, "receipt SHA-256")
    package.require(len(receipt_raw) == capture["bytes"] and package.digest(receipt_raw) == capture["sha256"],
                    "decoded receipt identity mismatch")
    receipt = bounded_json(receipt_raw, MAX_RECEIPT_BYTES, "CLI result")
    same(receipt, expected_result, "closed recorded CLI result")
    source = custody.inspect_source_package(
        source_raw, expected_package_sha256, expected_source_commit=expected_source_commit,
        expected_source_tree=expected_source_tree, expected_rights_sha256=expected_rights_sha256)
    inspection = source["source_package_inspection"]
    same({key: inspection[key] for key in (*pins, "material_files", "evidence_files", "tree_objects", "claim")},
         {key: receipt[key] for key in (*pins, "material_files", "evidence_files", "tree_objects", "claim")},
         "source reader and recorded CLI summary")
    return {"format": FORMAT, "status": "SOURCE_ONLY", "mode": "RECORDED_PUBLISHED_CLI_ONLY",
            "fresh_central_execution": "NOT_RUN", "central_commit": expected_central_commit,
            "central_tree": expected_central_tree, "profile_sha256": expected_profile_sha256,
            "source_package_sha256": expected_package_sha256,
            "recorded_artifacts": profile["artifacts"], "recorded_receipt_sha256": package.digest(receipt_raw),
            "central_result": receipt, "summary_agreement": True,
            "central_evidence_byte_equivalence": "NOT_ESTABLISHED",
            "local_original_evidence": source["original_evidence"],
            "source_custody": source, "authority": False, "enrollment_credit": 0,
            "accepted_obligations": 0, "production_obligations": "ALL_SIX_OPEN",
            "source_adapter_cutover": False, "source_retirement": False,
            "target_exclusive_custody": None, "Host_admission": None, "current_production_rights": None}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for flag in ("profile", "packet-dir", "package"):
        parser.add_argument("--" + flag, type=Path, required=True)
    for pin in ("profile-sha256", "central-commit", "central-tree", "package-sha256",
                "source-commit", "source-tree", "rights-sha256"):
        parser.add_argument("--expect-" + pin, required=True)
    argv = list(sys.argv[1:] if argv is None else argv)
    flags = [value.split("=", 1)[0] for value in argv if value.startswith("--")]
    if len(set(flags)) != len(flags):
        parser.error("every CLI option must occur exactly once")
    args = parser.parse_args(argv)
    try:
        profile_raw = package.regular(args.profile, MAX_PROFILE_BYTES)
        package.pin(args.expect_profile_sha256, 64, "profile SHA-256")
        package.require(package.digest(profile_raw) == args.expect_profile_sha256, "independent profile pin mismatch")
        profile = bounded_json(profile_raw, MAX_PROFILE_BYTES, "profile")
        package.require(type(profile) is dict and type(profile.get("artifacts")) is dict,
                        "profile artifact mapping required")
        package.keys(profile["artifacts"], ("interoperability", "cli_capture", "cli_log"), "profile artifacts")
        artifacts = {}
        for role, row in profile["artifacts"].items():
            package.require(type(row) is dict and "path" in row, "artifact path required")
            package.safe_path(row["path"])
            artifacts[role] = package.regular(args.packet_dir / row["path"], MAX_ARTIFACT_BYTES)
        result = inspect_recorded_package(
            package.regular(args.package, package.MAX_PACKAGE_BYTES), args.expect_package_sha256,
            profile_raw=profile_raw, expected_profile_sha256=args.expect_profile_sha256, artifacts=artifacts,
            expected_central_commit=args.expect_central_commit, expected_central_tree=args.expect_central_tree,
            expected_source_commit=args.expect_source_commit, expected_source_tree=args.expect_source_tree,
            expected_rights_sha256=args.expect_rights_sha256)
        sys.stdout.buffer.write(package.encode(result))
    except (ValueError, OSError, KeyError, TypeError) as exc:
        parser.exit(2, "recorded selected packet refused: " + str(exc) + "\n")


if __name__ == "__main__":
    main()
