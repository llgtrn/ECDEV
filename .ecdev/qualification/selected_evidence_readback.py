"""Check three raw evidence responses against original pinned source payloads.

This API performs no filesystem, network, Git or subprocess operations. Response
values alone do not authenticate a process, binary or source build. The caller's
actual qualification receipt must independently bind the invoked process to its
reviewed build identity. Byte equality grants no production authority or rights.
"""

import price_custody as custody
import price_source_package as package


MAX_STDOUT_BYTES = 4 * 1024 * 1024
EVIDENCE = {
    package.WITNESS_PATH: {"bytes": 2694156, "sha256": package.WITNESS_SHA256},
    package.RIGHTS_PATH: {"bytes": 46314, "sha256": package.RIGHTS_SHA256},
    package.PREREQUISITE_PATH: {"bytes": 152286, "sha256": package.PREREQUISITE_SHA256},
}


def inspect_readback(package_raw, expected_package_sha256, *, expected_source_commit,
                     expected_source_tree, expected_rights_sha256, responses):
    """Verify complete source membership once, then compare exact raw responses.

    All four pins must be obtained independently of package and response claims.
    ``responses`` is exactly the three canonical evidence paths, each mapped to
    an exact ``{exit: int, stdout: bytes, stderr: bytes}`` dictionary. This method
    neither invokes nor authenticates the process that supplied those values.
    """
    for value, length, label in ((expected_package_sha256, 64, "package SHA-256"),
                                 (expected_source_commit, 40, "source commit"),
                                 (expected_source_tree, 40, "source tree"),
                                 (expected_rights_sha256, 64, "rights SHA-256")):
        package.pin(value, length, label)
    source = custody.inspect_source_package(
        package_raw, expected_package_sha256, expected_source_commit=expected_source_commit,
        expected_source_tree=expected_source_tree, expected_rights_sha256=expected_rights_sha256)
    package.keys(responses, EVIDENCE, "raw evidence responses")
    package.require(all(type(path) is str for path in responses), "canonical evidence path strings required")
    # The unchanged custody reader has already verified every original record,
    # its independent digest, and complete package membership. Retain those raw
    # payloads from the validated outer package; never parse/reserialize them.
    value = package.strict_json(package_raw)
    original = {row["path"]: bytes.fromhex(row["content_hex"]) for row in value["evidence"]}
    package.keys(original, EVIDENCE, "original evidence")
    for path, expected in EVIDENCE.items():
        response = responses[path]
        package.keys(response, ("exit", "stdout", "stderr"), "raw evidence response")
        package.require(type(response["exit"]) is int and response["exit"] == 0,
                        "raw evidence process did not succeed")
        package.require(type(response["stderr"]) is bytes and response["stderr"] == b"",
                        "raw evidence stderr must be empty bytes")
        raw = response["stdout"]
        package.require(type(raw) is bytes and len(raw) <= MAX_STDOUT_BYTES,
                        "raw evidence stdout must be bounded bytes")
        package.require(len(raw) == expected["bytes"]
                        and package.digest(raw) == expected["sha256"],
                        "raw evidence length or independent SHA-256 mismatch")
        package.require(len(original[path]) == expected["bytes"]
                        and package.digest(original[path]) == expected["sha256"]
                        and raw == original[path], "raw evidence differs from retained original bytes")
    return {
        "format": "ecdev-selected-evidence-readback-v1",
        "status": "SOURCE_ONLY",
        "scope": "THREE_ORIGINAL_PINNED_EVIDENCE_PAYLOADS_ONLY",
        "byte_equivalent_payloads": 3,
        "evidence": {path: dict(expected) for path, expected in EVIDENCE.items()},
        "source_package_inspection": source["source_package_inspection"],
        "process_identity": "NOT_AUTHENTICATED_BY_RESPONSES",
        "missing_target_evidence": source["missing_target_evidence"],
        "target_execution": "NOT_RUN",
        "production_obligations": "ALL_SIX_OPEN",
        "authority": False,
        "canonical_enrollment": "BLOCKED",
        "enrollment_credit": 0,
        "accepted_obligations": 0,
        "whole_capture": False,
        "all828_admitted_mappings": 0,
        "runtime_admission": "NO_GRANT",
        "source_adapter_cutover": False,
        "source_retirement": False,
        "target_exclusive_custody": None,
        "Host_admission": None,
        "current_production_rights": None,
    }
