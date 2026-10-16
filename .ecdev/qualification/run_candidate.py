#!/usr/bin/env python3
"""Qualify a current source-eight candidate, retaining the unchanged custody qualification.

All writes go to a new disposable output. Local Git membership and reader parity establish
neither canonical enrollment nor rights, native acceptance, or runtime-data migration.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys


spec = importlib.util.spec_from_file_location("ecdev_custody_runner", Path(__file__).with_name("run.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)

CANDIDATE_SOURCE = ".ecdev/qualification/current_candidate.rs"
REQUIRED_INPUTS = (CANDIDATE_SOURCE, ".ecdev/qualification/run_candidate.py",
                   ".ecdev/qualification/test_run_candidate.py")
MAPPING = "source-eight-mapping-candidate-v1.json"
INSPECTION = "source-eight-mapping-inspection-v1.json"
CHECKS = (
    "caller_bound_current_mapping_roundtrip",
    "all_source_node_ids_and_metadata_preserved",
    "all_four_payload_lanes_match_central_bytes",
    "all_rights_and_foreign_semantics_preserved",
    "current_global_hash_candidates_match_published_rule",
    "stale_capture_pin_refused",
    "target_self_grant_refused",
    "matching_protocol_lock_does_not_enroll_source",
)
PINNED_SCHEMA_DIGEST = "sha256:4e3b2d05c3970ed4fa2a06b5a2430a5289301cf1e455789c3f2f88ed0cff0abc"
MAX_FILE = 64 * 1024 * 1024
MAX_TOTAL = 512 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def strict_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result

    def invalid(value):
        raise ValueError(f"nonfinite JSON value: {value}")

    return json.loads(data, object_pairs_hook=unique, parse_constant=invalid)


def safe_relative(path):
    if (not isinstance(path, str) or not path or path.startswith("/") or "\\" in path
            or any(ord(c) < 32 or ord(c) == 127 for c in path)
            or any(part in ("", ".", "..") for part in path.split("/"))):
        raise ValueError("unsafe source path")
    return path


def git_environment():
    env = {key: os.environ[key] for key in ("PATH", "HOME") if key in os.environ}
    env.update(GIT_NO_REPLACE_OBJECTS="1", GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0",
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
               GIT_CONFIG_SYSTEM=os.devnull, GIT_OPTIONAL_LOCKS="0")
    return env


def git(source, *args, input_bytes=None):
    return subprocess.run(
        ["git", "--git-dir", str(source / ".git"), "-c", "protocol.allow=never",
         "-c", "core.fsmonitor=false", *args], env=git_environment(),
        input=input_bytes, check=True, capture_output=True,
    ).stdout


def rows_of(binding):
    if not isinstance(binding, dict) or not re.fullmatch("[0-9a-f]{40}", binding.get("observed_head", "")):
        raise ValueError("invalid observed source HEAD")
    rows = binding.get("inputs")
    if not isinstance(rows, list) or not rows:
        raise ValueError("source input rows are missing")
    by_path = {}
    total = 0
    for row in rows:
        if not isinstance(row, dict) or set(row) != {"path", "sha256", "bytes", "executable"}:
            raise ValueError("invalid source input row")
        path = safe_relative(row["path"])
        if (path in by_path or not isinstance(row["sha256"], str)
                or not re.fullmatch("[0-9a-f]{64}", row["sha256"])
                or type(row["bytes"]) is not int or not 0 <= row["bytes"] <= MAX_FILE
                or type(row["executable"]) is not bool):
            raise ValueError("invalid or duplicate source input binding")
        total += row["bytes"]
        if total > MAX_TOTAL or len(by_path) >= 100_000:
            raise ValueError("source inputs exceed membership budget")
        by_path[path] = row
    if list(by_path) != sorted(by_path) or binding.get("index_paths") != sorted(by_path):
        raise ValueError("source input rows must exactly equal sorted index paths")
    if binding.get("manifest_sha256") != digest(canonical(rows)):
        raise ValueError("source input manifest digest mismatch")
    if any(path not in by_path for path in REQUIRED_INPUTS):
        raise ValueError("candidate programs must be indexed initial source inputs")
    return by_path


def parse_tree(data):
    entries = {}
    for record in data.split(b"\0"):
        if not record:
            continue
        try:
            header, raw_path = record.split(b"\t", 1)
            mode, kind, blob = header.decode("ascii").split()
            path = safe_relative(raw_path.decode("utf-8"))
        except (ValueError, UnicodeError) as error:
            raise ValueError("invalid committed source tree entry") from error
        if (mode not in ("100644", "100755") or kind != "blob"
                or not re.fullmatch("[0-9a-f]{40}", blob) or path in entries):
            raise ValueError("unsupported or duplicate committed source tree entry")
        entries[path] = (mode, blob)
    return entries


def committed_blobs(source, entries, rows):
    expected = {}
    for path, (_mode, blob) in entries.items():
        size = rows[path]["bytes"]
        if blob in expected and expected[blob] != size:
            raise ValueError("one blob has conflicting source lengths")
        expected[blob] = size
    objects = sorted(expected)
    data = git(source, "cat-file", "--batch", input_bytes="".join(f"{oid}\n" for oid in objects).encode())
    if len(data) > MAX_TOTAL + len(objects) * 100:
        raise ValueError("committed blob batch exceeds budget")
    result = {}
    offset = 0
    for oid in objects:
        end = data.find(b"\n", offset)
        if end < 0:
            raise ValueError("truncated committed blob header")
        header = data[offset:end].split()
        if len(header) != 3 or header[:2] != [oid.encode(), b"blob"] or not header[2].isdigit():
            raise ValueError("missing or invalid committed blob")
        size = int(header[2])
        if size != expected[oid]:
            raise ValueError("committed blob length mismatch")
        start = end + 1
        content = data[start:start + size]
        offset = start + size + 1
        if len(content) != size or data[offset - 1:offset] != b"\n":
            raise ValueError("truncated committed blob body")
        if hashlib.sha1(b"blob " + str(size).encode() + b"\0" + content).hexdigest() != oid:
            raise ValueError("committed blob object identity mismatch")
        result[oid] = content
    if offset != len(data):
        raise ValueError("extra committed blob data")
    return result


def git_membership(source, binding):
    """Bind every initially selected input to the same committed tree and current file bytes."""
    rows = rows_of(binding)
    head = binding["observed_head"]
    if git(source, "rev-parse", "HEAD^{commit}").decode().strip() != head:
        raise ValueError("source HEAD changed before membership verification")
    indexed = git(source, "ls-files", "--cached", "--full-name", "-z").decode().split("\0")
    if sorted(path for path in indexed if path) != list(rows):
        raise ValueError("current index paths differ from initial source inputs")
    tree = git(source, "rev-parse", f"{head}^{{tree}}").decode().strip()
    if not re.fullmatch("[0-9a-f]{40}", tree):
        raise ValueError("invalid committed source tree identity")
    entries = parse_tree(git(source, "ls-tree", "-r", "-z", "--full-tree", head))
    if set(entries) != set(rows):
        raise ValueError("committed paths differ from complete initial input set")
    blobs = committed_blobs(source, entries, rows)
    manifest = []
    by_path = {}
    for path, row in rows.items():
        mode, blob = entries[path]
        content = blobs[blob]
        file = base.physical_path(source / path)
        metadata = file.stat()
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError(f"source input is not regular: {path}")
        actual_mode = "100755" if metadata.st_mode & 0o111 else "100644"
        if (digest(content) != row["sha256"] or len(content) != row["bytes"]
                or (mode == "100755") != row["executable"] or actual_mode != mode
                or file.read_bytes() != content):
            raise ValueError(f"committed/input/worktree bytes or mode differ: {path}")
        manifest.append({"path": path, "mode": mode, "blob": blob,
                         "sha256": row["sha256"], "bytes": row["bytes"]})
        by_path[path] = content
    if git(source, "rev-parse", "HEAD^{commit}").decode().strip() != head:
        raise ValueError("source HEAD changed during membership verification")
    return ({"format": "source-eight-git-membership-v1", "source_repository": "llgtrn/ECDEV",
             "source_commit": head, "source_tree": tree,
             "initial_input_manifest_sha256": binding["manifest_sha256"],
             "manifest_sha256": digest(canonical(manifest)), "inputs": manifest,
             "scope": "ALL_INDEX_SELECTED_INPUTS_MATCH_COMMITTED_BLOBS_AND_WORKTREE; LOCAL_GIT_CUSTODY_ONLY",
             "runtime_data": "UNTRACKED_AND_IGNORED_RUNTIME_DATA_PRESERVED_IN_PLACE; NOT_ENUMERATED; NOT_MIGRATED",
             "history_completeness": "NOT_ATTESTED", "rights_authorization": None}, by_path)


def capture_membership(data, expected_digest, head, committed):
    if digest(data) != expected_digest:
        raise ValueError("current capture digest mismatch")
    capture = strict_json(data)
    if (not isinstance(capture, dict) or capture.get("format") != "ecdev-source-capture-v1"
            or capture.get("binding") != "CAPTURED_BYTES; HEAD_OBSERVED_ONLY; NOT_ENROLLED; NOT_REVERIFIED"
            or capture.get("source_id") != "ecdev" or capture.get("source_repository") != "llgtrn/ECDEV"
            or capture.get("observed_head") != head):
        raise ValueError("capture source identity or observed HEAD mismatch")
    files = capture.get("files")
    if not isinstance(files, list) or len(files) != 734:
        raise ValueError("current candidate must preserve all 734 captured payloads")
    seen = set()
    expected_paths = {path for path in committed
                      if any(path.startswith(f".ecdev/{lane}/")
                             for lane in ("declared", "history", "evidence", "knowledge"))}
    for row in files:
        if not isinstance(row, dict) or set(row) != {"path", "sha256", "hex"}:
            raise ValueError("invalid captured payload")
        path = safe_relative(row["path"])
        if path in seen or path not in committed:
            raise ValueError("captured payload is duplicated or not committed")
        seen.add(path)
        if not isinstance(row["hex"], str) or not re.fullmatch("(?:[0-9a-f]{2})*", row["hex"]):
            raise ValueError("noncanonical captured payload hex")
        content = bytes.fromhex(row["hex"])
        if row["sha256"] != digest(content) or content != committed[path]:
            raise ValueError("captured payload differs from committed blob")
    if seen != expected_paths:
        raise ValueError("capture does not enumerate the complete committed source payload lanes")
    return capture


def read_registered_source_ids(reference, schema_digest):
    """Read the actual registry of the already byte-qualified engine; install no source state."""
    if schema_digest != PINNED_SCHEMA_DIGEST:
        raise ValueError("source registry requires the exact qualified schema digest")
    path = base.physical_path(reference / "core/semantic/src/schema.rs")
    if not path.is_file() or path.stat().st_size > 1024 * 1024:
        raise ValueError("qualified source registry is missing or oversized")
    text = path.read_text()
    declarations = list(re.finditer(r"\bpub\s+const\s+SHARDS\s*:\s*&\s*\[\s*Shard\s*\]\s*=\s*&\s*\[", text))
    if len(declarations) != 1:
        raise ValueError("qualified source registry must have exactly one SHARDS declaration")
    tail = text[declarations[0].end():]
    end = tail.find("];")
    if end < 0:
        raise ValueError("qualified source registry declaration is incomplete")
    body = tail[:end]
    entry = re.compile(r'\s*Shard\s*\{\s*id\s*:\s*"([a-z][a-z0-9]*(?:-[a-z0-9]+)*)"\s*,\s*'
                       r'origin\s*:\s*"[^"\\\x00-\x1f]*"\s*,\s*domain\s*:\s*"[^"\\\x00-\x1f]*"\s*,\s*\}\s*,')
    ids = []
    offset = 0
    while body[offset:].strip():
        matched = entry.match(body, offset)
        if matched is None:
            raise ValueError("qualified source registry has unsupported SHARDS grammar")
        ids.append(matched[1])
        offset = matched.end()
    if len(ids) != 7 or len(set(ids)) != 7 or "ecdev" in ids:
        raise ValueError("qualified source registry must retain seven unique prior sources")
    return ids


def validate_candidate(report, capture_digest, output, registered_source_ids):
    expected = {"format": "ecdev-source-eight-candidate-v1", "status": "PASS",
                "source_id": "ecdev", "source_repository": "llgtrn/ECDEV",
                "capture_sha256": "sha256:" + capture_digest, "canonical_enrollment": "BLOCKED",
                "source_lock_installed": False, "registry_binding": "PENDING",
                "registered_sources": 7, "source_registered": False,
                "tests_passed": 8, "tests_failed": 0,
                "current_rights_authorization": None, "target_mapping": None,
                "target_custody": None, "proposed_target": None}
    if not isinstance(report, dict) or any(type(report.get(k)) is not type(v) or report[k] != v
                                           for k, v in expected.items()):
        raise ValueError("candidate result does not match blocked source-eight contract")
    if (not isinstance(registered_source_ids, list) or len(registered_source_ids) != 7
            or any(not isinstance(source_id, str) for source_id in registered_source_ids)
            or len(set(registered_source_ids)) != 7 or "ecdev" in registered_source_ids):
        raise ValueError("candidate requires the complete qualified seven-source registry")
    registry_expected = {
        "current_registered_source_ids": registered_source_ids,
        "required_closed_source_ids": sorted((*registered_source_ids, "ecdev")),
        "source_id": "ecdev", "source_repository": "llgtrn/ECDEV",
        "canonical_source_owner": None, "registry_entry_accepted": False, "new_common_lock": None,
    }
    registry = report.get("candidate_registry_expectation")
    if (not isinstance(registry, dict) or set(registry) != set(registry_expected)
            or any(type(registry[k]) is not type(v) or registry[k] != v
                   for k, v in registry_expected.items())):
        raise ValueError("candidate registry expectation must preserve exact seven/eight source sets and pending authority")
    checks = report.get("checks")
    if (not isinstance(checks, list) or len(checks) != len(CHECKS)
            or any(not isinstance(row, dict) or row.get("pass") is not True for row in checks)
            or [row.get("name") for row in checks] != list(CHECKS)):
        raise ValueError("candidate checks are incomplete, duplicated, reordered or failed")
    artifacts = {}
    for filename, field in ((MAPPING, "mapping_sha256"), (INSPECTION, "inspection_sha256")):
        path = base.physical_path(output / filename)
        if not path.is_file() or path.stat().st_size > 16 * 1024 * 1024:
            raise ValueError("candidate artifact is missing or oversized")
        data = path.read_bytes()
        if report.get(field) != "sha256:" + digest(data):
            raise ValueError("candidate artifact digest mismatch")
        artifacts[filename] = {"sha256": digest(data), "bytes": len(data)}
    mapping = strict_json((output / MAPPING).read_bytes())
    if (not isinstance(mapping, dict) or mapping.get("target_custody", "missing") is not None
            or not isinstance(mapping.get("node_mappings"), list) or not mapping["node_mappings"]
            or any(not isinstance(row, dict) or row.get("proposed_target", "missing") is not None
                   for row in mapping.get("node_mappings", []))):
        raise ValueError("candidate artifact cannot self-grant target custody or mapping")
    return artifacts


def write_new(path, data):
    with path.open("xb") as stream:
        stream.write(data)


def run(reference, objects, output, cargo, toolchain):
    source = Path(__file__).resolve().parents[2]
    reference, objects, output = (base.physical_path(path) for path in (reference, objects, output))
    base.run(reference, objects, output, cargo, toolchain)
    base_bytes = (output / "report.json").read_bytes()
    report = strict_json(base_bytes)
    contract = report["qualification_contract"]
    base.validate_report(report, contract)
    if report["tests_passed"] != 62:
        raise ValueError("unchanged base custody qualification must execute exactly 62 checks")
    binding = report["source_input_binding"]
    membership, committed = git_membership(source, binding)
    if contract != strict_json(committed["research/commerce/governance/central-reader-contract-v1.json"]):
        raise ValueError("base qualification contract differs from committed source input")
    if base.verify_reference(reference, objects, contract) != report["engine_input_binding"]:
        raise ValueError("engine reference changed before source registry reading")
    registered_source_ids = read_registered_source_ids(reference, contract["schema_digest"])
    capture_digest = report["fresh"]["sha256"]
    if not isinstance(capture_digest, str) or not re.fullmatch("sha256:[0-9a-f]{64}", capture_digest):
        raise ValueError("invalid base fresh capture digest")
    capture_digest = capture_digest[7:]
    if report.get("fresh_artifact") != "committed-source-capture-v1.json":
        raise ValueError("unexpected base capture artifact")
    capture_bytes = (output / report["fresh_artifact"]).read_bytes()
    capture_membership(capture_bytes, capture_digest, binding["observed_head"], committed)
    write_new(output / "source-eight-git-membership-v1.json", json.dumps(membership, indent=2).encode() + b"\n")
    project = output / "harness"
    (project / "src/bin").mkdir()
    write_new(project / "src/bin/current_candidate.rs", committed[CANDIDATE_SOURCE])
    env = base.build_environment(project, output)
    commands = [
        [cargo, f"+{toolchain}", "clippy", "--offline", "--frozen", "--all-targets",
         "--manifest-path", str(project / "Cargo.toml"), "--", "-D", "warnings"],
        [cargo, f"+{toolchain}", "run", "--offline", "--frozen", "--manifest-path", str(project / "Cargo.toml"),
         "--bin", "current_candidate", "--", str(source), str(output), capture_digest],
    ]
    with (output / "candidate-cargo.log").open("xb") as log:
        for command in commands:
            result = subprocess.run(command, env=env, cwd=project, stdout=subprocess.PIPE, stderr=log, check=False)
            if result.returncode:
                write_new(output / "candidate-failed-command-stdout", result.stdout)
                raise ValueError("candidate command failed; see candidate-cargo.log")
    write_new(output / "unvalidated-candidate-output.json", result.stdout)
    candidate = strict_json(result.stdout)
    artifacts = validate_candidate(candidate, capture_digest, output, registered_source_ids)
    if base.verify_reference(reference, objects, contract) != report["engine_input_binding"]:
        raise ValueError("engine reference changed during candidate execution")
    if base.source_binding(source, (row["path"] for row in binding["inputs"])) != binding:
        raise ValueError("source inputs changed during candidate execution")
    repeated, _ = git_membership(source, binding)
    if repeated != membership or (output / "report.json").read_bytes() != base_bytes:
        raise ValueError("committed membership or base qualification changed")
    if (output / "committed-source-capture-v1.json").read_bytes() != capture_bytes:
        raise ValueError("capture changed during candidate execution")
    wrapped = {"format": "ecdev-source-eight-candidate-qualification-v1", "status": "PASS",
               "canonical_enrollment": "BLOCKED", "registry_binding": "PENDING",
               "source_registered": False, "source_lock_installed": False,
               "current_rights_authorization": None, "target_mapping": None,
               "base_custody_checks_passed": 62, "candidate_checks_passed": 8,
               "base_report_sha256": digest(base_bytes), "base_report": report,
               "candidate_report": candidate, "candidate_artifacts": artifacts,
               "git_membership": membership, "capture_sha256": "sha256:" + capture_digest,
               "candidate_source_sha256": digest(committed[CANDIDATE_SOURCE]),
               "runner_sha256": digest(Path(__file__).read_bytes()),
               "candidate_binary_sha256": digest((output / "target/debug/current_candidate").read_bytes()),
               "execution_commands": commands,
               "binding_trust": "LOCAL_IMMUTABLE_GIT_OBJECTS_AND_TRUSTED_FILESYSTEM; NOT_AUTHENTICATED_EXECUTION"}
    path = output / "source-eight-qualification-report-v1.json"
    write_new(path, json.dumps(wrapped, indent=2).encode() + b"\n")
    print(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--objects", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--toolchain", default="1.87.0")
    args = parser.parse_args()
    try:
        run(args.reference, args.objects, args.output, args.cargo, args.toolchain)
    except (ValueError, OSError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"source-eight candidate refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
