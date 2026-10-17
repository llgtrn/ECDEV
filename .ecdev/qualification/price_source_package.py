#!/usr/bin/env python3
"""Export selected source bytes; verify their pinned membership offline with stdlib.

The inspector needs independently obtained package SHA-256, source commit/tree,
and published rights appendix SHA-256. A package's own claims are not trust pins.
Git SHA-1 proves membership under those pins, not title or operational authority.
Only export reads a checkout or invokes Git. No packaged material is executed.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys


FORMAT = "ecdev-selected-price-source-package-v1"
RIGHTS_PATH = "research/commerce/governance/price-selected-source-rights-provenance-v1.json"
RIGHTS_SHA256 = "c3bfc1ccf9a84d950cd3cf663f6f2062c2470b22515ab877ed6b1563d9cd6410"
WITNESS_PATH = "research/commerce/governance/price-selected-source-custody-db92bf27-v1.json"
WITNESS_SHA256 = "e6f58d934cc1b55d949dc925f83510f5361432f64a3b5620424da3a01bc8d2b0"
PREREQUISITE_PATH = "research/commerce/governance/price-source-parity-cutover-prerequisites-v1.json"
PREREQUISITE_SHA256 = "6b2a1a93dfe79a503999650826b0d93c6442b022f77380a6738216a515974560"
EVIDENCE = {RIGHTS_PATH: RIGHTS_SHA256, WITNESS_PATH: WITNESS_SHA256,
            PREREQUISITE_PATH: PREREQUISITE_SHA256}
MAX_PACKAGE_BYTES = 24 * 1024 * 1024
MAX_FILE_BYTES = 4 * 1024 * 1024
MAX_TOTAL_BYTES = 10 * 1024 * 1024
MAX_TREE_BYTES = 1024 * 1024
MAX_TREES = 128
MAX_TREE_ENTRIES = 20000
MAX_JSON_NODES = 250000
MAX_JSON_DEPTH = 64
STATUS = {"source_authority": "SOURCE_ONLY", "current_production_rights": None,
          "canonical_enrollment": "BLOCKED", "target_exclusive_custody": None,
          "Host_admission": None, "source_adapter_cutover": False,
          "source_retirement": False, "production_obligations_closed": 0,
          "context_nodes_adopted": 0, "all828_admitted_mappings": 0}
CLAIM = "PARTIAL_SELECTED_SOURCE_BYTES_AND_LOCAL_GIT_MEMBERSHIP_ONLY"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def git_digest(kind, raw):
    return hashlib.sha1(kind.encode("ascii") + b" " + str(len(raw)).encode("ascii")
                        + b"\0" + raw).hexdigest()


def pin(value, length, label):
    require(type(value) is str and re.fullmatch("[0-9a-f]{%d}" % length, value) is not None,
            label + " must be an independent lowercase hex pin")
    return value


def encode(value):
    return (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True, allow_nan=False)
            + "\n").encode("ascii")


def strict_json(raw):
    require(type(raw) is bytes and len(raw) <= MAX_PACKAGE_BYTES, "JSON byte budget exceeded")

    def pairs(rows):
        result = {}
        for key, value in rows:
            require(key not in result, "duplicate JSON key")
            result[key] = value
        return result

    def nonfinite(value):
        raise ValueError("nonfinite JSON number: " + value)

    try:
        value = json.loads(raw, object_pairs_hook=pairs, parse_constant=nonfinite)
    except (UnicodeError, RecursionError, json.JSONDecodeError) as exc:
        raise ValueError("invalid JSON") from exc
    pending = [(value, 0)]
    nodes = 0
    while pending:
        item, depth = pending.pop()
        nodes += 1
        require(nodes <= MAX_JSON_NODES and depth <= MAX_JSON_DEPTH, "JSON node/depth budget exceeded")
        if type(item) is dict:
            pending.extend((v, depth + 1) for v in item.values())
        elif type(item) is list:
            pending.extend((v, depth + 1) for v in item)
        elif type(item) is float:
            require(math.isfinite(item), "nonfinite JSON number")
    return value


def keys(value, expected, label):
    require(type(value) is dict and set(value) == set(expected), label + " fields changed")


def count(value, maximum, label):
    require(type(value) is int and 0 <= value <= maximum, label + " invalid or exceeds budget")
    return value


def safe_path(value):
    require(type(value) is str and 0 < len(value) <= 1024 and "\\" not in value,
            "unsafe path")
    parts = value.split("/")
    require(len(parts) <= 32 and all(p not in ("", ".", "..")
                                   and not any(ord(c) < 32 or ord(c) == 127 for c in p)
                                   for p in parts), "unsafe path")
    require(not re.match("^[A-Za-z]:", value), "drive path is not portable")
    try:
        value.encode("utf-8")
    except UnicodeError as exc:
        raise ValueError("path is not valid UTF-8") from exc
    return parts


def object_record(oid, raw):
    return {"oid": oid, "bytes": len(raw), "content_hex": raw.hex()}


def raw_record(record, maximum, label, object_kind=None):
    n = count(record["bytes"], maximum, label + " byte count")
    value = record["content_hex"]
    require(type(value) is str and len(value) == n * 2
            and re.fullmatch("[0-9a-f]*", value) is not None, label + " invalid byte encoding")
    raw = bytes.fromhex(value)
    if object_kind:
        pin(record["oid"], 40, label + " object")
        require(git_digest(object_kind, raw) == record["oid"], label + " object hash mismatch")
    return raw


def parse_tree(raw):
    """Parse a full raw Git tree; sibling objects need not be exported."""
    entries = {}
    offset = 0
    previous = None
    while offset < len(raw):
        space, nul = raw.find(b" ", offset), raw.find(b"\0", offset)
        require(offset < space < nul and nul + 21 <= len(raw), "malformed raw tree")
        mode, name = raw[offset:space], raw[space + 1:nul]
        require(mode in (b"40000", b"100644", b"100755", b"120000", b"160000"),
                "invalid tree mode")
        require(name and name not in (b".", b"..") and b"/" not in name
                and b"\\" not in name and not any(c < 32 or c == 127 for c in name),
                "unsafe tree name")
        require(name not in entries, "duplicate tree name")
        sort_key = name + (b"/" if mode == b"40000" else b"\0")
        require(previous is None or previous < sort_key, "noncanonical tree order")
        previous = sort_key
        entries[name] = (mode.decode("ascii"), raw[nul + 1:nul + 21].hex())
        require(len(entries) <= MAX_TREE_ENTRIES, "tree entry budget exceeded")
        offset = nul + 21
    return entries


def commit_tree(raw):
    header, separator, _ = raw.partition(b"\n\n")
    require(separator and header.startswith(b"tree "), "invalid commit headers")
    lines = [line for line in header.split(b"\n") if line.startswith(b"tree ")]
    require(len(lines) == 1, "commit must have exactly one root tree")
    try:
        return pin(lines[0][5:].decode("ascii"), 40, "commit root tree")
    except UnicodeError as exc:
        raise ValueError("invalid commit root tree") from exc


def membership(path, trees, root_tree, used):
    parts = safe_path(path)
    oid = root_tree
    for i, part in enumerate(parts):
        require(oid in trees, "missing ancestor tree for " + path)
        used.add(oid)
        item = trees[oid].get(part.encode("utf-8"))
        require(item is not None, "path absent from source tree: " + path)
        mode, oid = item
        if i < len(parts) - 1:
            require(mode == "40000", "non-directory path ancestor")
        else:
            require(mode in ("100644", "100755"), "selected path must be a regular Git blob")
    return mode, oid


def file_record(path, raw, mode):
    return {"path": path, "sha256": digest(raw), "bytes": len(raw),
            "git_blob": git_digest("blob", raw), "mode": mode, "content_hex": raw.hex()}


def file_records(rows, expected_count):
    require(type(rows) is list and len(rows) == expected_count, "file record count changed")
    result, total = {}, 0
    for row in rows:
        keys(row, ("path", "sha256", "bytes", "git_blob", "mode", "content_hex"), "file record")
        safe_path(row["path"])
        require(row["path"] not in result, "duplicate file path")
        raw = raw_record(row, MAX_FILE_BYTES, "file")
        pin(row["sha256"], 64, "file SHA-256")
        pin(row["git_blob"], 40, "file Git blob")
        require(digest(raw) == row["sha256"] and git_digest("blob", raw) == row["git_blob"],
                "file hash mismatch")
        require(row["mode"] in ("100644", "100755"), "file mode is not regular")
        result[row["path"]] = (row, raw)
        total += len(raw)
        require(total <= MAX_TOTAL_BYTES, "file total byte budget exceeded")
    require([row["path"] for row in rows] == sorted(result), "file records must be path sorted")
    return result, total


def inspect_package_evidence(raw, expected_package_sha256, *, expected_source_commit,
                             expected_source_tree, expected_rights_sha256):
    """Return inspection and exact evidence bytes after one complete offline check.

    This function reads no checkout and invokes neither Git nor packaged code.
    The evidence mapping contains only the three immutable published documents.
    """
    pin(expected_package_sha256, 64, "package SHA-256")
    pin(expected_source_commit, 40, "source commit")
    pin(expected_source_tree, 40, "source tree")
    pin(expected_rights_sha256, 64, "published rights SHA-256")
    require(expected_rights_sha256 == RIGHTS_SHA256, "published rights appendix pin changed")
    require(type(raw) is bytes and len(raw) <= MAX_PACKAGE_BYTES, "package byte budget exceeded")
    require(digest(raw) == expected_package_sha256, "independent package SHA-256 mismatch")
    value = strict_json(raw)
    keys(value, ("format", "source_commit", "source_tree", "commit", "trees", "materials",
                 "evidence", "claim", "status"), "package")
    require(raw == encode(value), "package must use canonical encoding without trailing data")
    require(value["format"] == FORMAT and value["claim"] == CLAIM, "package claim changed")
    require(encode(value["status"]) == encode(STATUS), "source authority/rights status changed")
    require(value["source_commit"] == expected_source_commit
            and value["source_tree"] == expected_source_tree, "independent source pins mismatch")
    keys(value["commit"], ("oid", "bytes", "content_hex"), "commit")
    commit = raw_record(value["commit"], MAX_TREE_BYTES, "commit", "commit")
    require(value["commit"]["oid"] == expected_source_commit
            and commit_tree(commit) == expected_source_tree, "commit/tree membership mismatch")
    rows = value["trees"]
    require(type(rows) is list and 0 < len(rows) <= MAX_TREES, "tree object count invalid")
    trees, tree_total, entry_total = {}, 0, 0
    for row in rows:
        keys(row, ("oid", "bytes", "content_hex"), "tree")
        tree = raw_record(row, MAX_TREE_BYTES, "tree", "tree")
        require(row["oid"] not in trees, "duplicate tree object")
        trees[row["oid"]] = parse_tree(tree)
        tree_total += len(tree)
        entry_total += len(trees[row["oid"]])
        require(tree_total <= MAX_TREE_BYTES and entry_total <= MAX_TREE_ENTRIES,
                "aggregate tree budget exceeded")
    require([r["oid"] for r in rows] == sorted(trees), "tree objects must be sorted")
    materials, material_total = file_records(value["materials"], 83)
    evidence, evidence_total = file_records(value["evidence"], len(EVIDENCE))
    require(material_total + evidence_total + tree_total + len(commit) <= MAX_TOTAL_BYTES,
            "package decoded byte budget exceeded")
    require(set(evidence) == set(EVIDENCE) and not set(materials).intersection(evidence),
            "evidence/material membership changed")
    for path, sha in EVIDENCE.items():
        require(evidence[path][0]["sha256"] == sha, "immutable evidence bytes changed")
    rights = strict_json(evidence[RIGHTS_PATH][1])
    witness = strict_json(evidence[WITNESS_PATH][1])
    prerequisite = strict_json(evidence[PREREQUISITE_PATH][1])
    expected_materials = rights["current_source_material"]
    require(len(expected_materials) == 83 and len({r["path"] for r in expected_materials}) == 83,
            "rights material inventory changed")
    require(set(materials) == {r["path"] for r in expected_materials}, "exact83 material paths required")
    for expected in expected_materials:
        record = materials[expected["path"]][0]
        require(encode({k: record[k] for k in expected}) == encode(expected),
                "published rights material pin mismatch")
    used = set()
    for path, (record, _) in {**materials, **evidence}.items():
        require(membership(path, trees, expected_source_tree, used)
                == (record["mode"], record["git_blob"]), "Git path/mode/blob membership mismatch")
    require(used == set(trees), "extra or dangling tree objects")
    bindings = {row["role"]: row["binding"] for row in witness["selected_nodes"]}
    require(encode(bindings) == encode(rights["selected_source_bindings"]), "source bindings differ")
    require(encode(witness["parity_cutover_prerequisites"]) == encode(prerequisite),
            "embedded prerequisites differ")
    inspection = {"format": "ecdev-selected-price-source-package-inspection-v1",
            "status": "VALID_PORTABLE_SELECTED_SOURCE_PACKAGE; SOURCE_ONLY",
            "package_sha256": digest(raw), "source_commit": expected_source_commit,
            "source_tree": expected_source_tree, "rights_sha256": expected_rights_sha256,
            "material_files": len(materials), "material_bytes": material_total,
            "evidence_files": len(evidence), "tree_objects": len(trees),
            "claim": CLAIM, "selected_source_bindings": bindings,
            "historical_rights_status": witness["status"],
            "published_rights_status": rights["status"],
            "operational_authority": dict(STATUS),
            "partial_material_boundary": witness["material_boundary"]["classification"],
            "offline_git_or_checkout_required": False}
    return inspection, {path: item[1] for path, item in evidence.items()}


def inspect_package(raw, expected_package_sha256, *, expected_source_commit,
                    expected_source_tree, expected_rights_sha256):
    """Verify bytes without reading the source checkout, invoking Git or executing data."""
    return inspect_package_evidence(raw, expected_package_sha256,
                                    expected_source_commit=expected_source_commit,
                                    expected_source_tree=expected_source_tree,
                                    expected_rights_sha256=expected_rights_sha256)[0]


def verified_evidence(raw, expected_package_sha256, *, path, expected_source_commit,
                      expected_source_tree, expected_rights_sha256):
    """Return exact pinned evidence bytes only after complete offline verification.

    Existing source handoff readers can consume the original witness/prerequisite
    without trusting a new projection or needing a checkout. This function does
    not execute evidence or grant operational authority to its consumer.
    """
    require(type(path) is str and path in EVIDENCE, "only immutable evidence paths are accessible")
    _, evidence = inspect_package_evidence(raw, expected_package_sha256,
                                          expected_source_commit=expected_source_commit,
                                          expected_source_tree=expected_source_tree,
                                          expected_rights_sha256=expected_rights_sha256)
    return evidence[path]


def regular(path, maximum):
    """Read a bounded regular file, refusing symlinks in every existing component."""
    path = Path(path).absolute()
    for parent in reversed((path, *path.parents)):
        require(not parent.is_symlink(), "symlink input is forbidden")
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        require(stat.S_ISREG(info.st_mode) and info.st_size <= maximum, "input is not a bounded regular file")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            raw = stream.read(maximum + 1)
        require(len(raw) <= maximum, "input byte budget exceeded")
        return raw
    finally:
        os.close(fd)


def worktree_mode(path):
    info = Path(path).lstat()
    require(stat.S_ISREG(info.st_mode), "selected worktree path is not regular")
    return "100755" if info.st_mode & stat.S_IXUSR else "100644"


def git_object(root, kind, oid):
    """Read only raw objects: no filters, hooks, replacement objects or lazy fetch."""
    pin(oid, 40, "Git object")
    require(kind in ("commit", "tree"), "unsupported object request")
    executable = shutil.which("git")
    require(executable is not None, "Git is needed only for export")
    env = {"PATH": os.defpath, "LC_ALL": "C", "GIT_CONFIG_NOSYSTEM": "1",
           "GIT_CONFIG_GLOBAL": os.devnull, "GIT_NO_REPLACE_OBJECTS": "1",
           "GIT_NO_LAZY_FETCH": "1", "GIT_OPTIONAL_LOCKS": "0", "GIT_TERMINAL_PROMPT": "0"}
    args = [executable, "--no-replace-objects", "-C", str(root), "-c", "core.hooksPath=" + os.devnull,
            "-c", "core.fsmonitor=false", "-c", "protocol.allow=never", "cat-file"]
    size = subprocess.run([*args, "-s", oid], env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, timeout=30, check=False)
    require(size.returncode == 0 and re.fullmatch(b"[0-9]{1,10}\n", size.stdout) is not None,
            "unable to read bounded local Git object size")
    require(int(size.stdout) <= MAX_TREE_BYTES, "Git object byte budget exceeded")
    # A bounded temporary output avoids unbounded capture of an unexpected object.
    import tempfile
    with tempfile.TemporaryFile() as output:
        result = subprocess.run([*args, kind, oid], env=env, stdout=output, stderr=subprocess.PIPE,
                                timeout=30, check=False)
        require(result.returncode == 0, "unable to read local Git object")
        require(output.tell() <= MAX_TREE_BYTES, "Git object byte budget exceeded")
        output.seek(0)
        raw = output.read(MAX_TREE_BYTES + 1)
    require(git_digest(kind, raw) == oid, "local Git object hash mismatch")
    return raw


def build_package(root, *, expected_source_commit, expected_source_tree, expected_rights_sha256):
    """Export only exact published material pins, refusing changed worktree bytes."""
    pin(expected_source_commit, 40, "source commit")
    pin(expected_source_tree, 40, "source tree")
    pin(expected_rights_sha256, 64, "published rights SHA-256")
    require(expected_rights_sha256 == RIGHTS_SHA256, "published rights appendix pin changed")
    root = Path(root).absolute()
    require(root.is_dir() and root.resolve() == root, "physical source root required")
    evidence_raw = {path: regular(root / path, MAX_FILE_BYTES) for path in EVIDENCE}
    for path, sha in EVIDENCE.items():
        require(digest(evidence_raw[path]) == sha, "published evidence changed")
    rights = strict_json(evidence_raw[RIGHTS_PATH])
    commit = git_object(root, "commit", expected_source_commit)
    require(commit_tree(commit) == expected_source_tree, "independent source tree mismatch")
    trees, tree_rows = {}, {}

    def locate(path):
        oid = expected_source_tree
        parts = safe_path(path)
        for i, part in enumerate(parts):
            if oid not in trees:
                require(len(trees) < MAX_TREES, "tree object count exceeds budget")
                raw = git_object(root, "tree", oid)
                trees[oid], tree_rows[oid] = parse_tree(raw), object_record(oid, raw)
            item = trees[oid].get(part.encode("utf-8"))
            require(item is not None, "selected file absent from pinned source tree")
            mode, oid = item
            require(mode in (("40000",) if i < len(parts) - 1 else ("100644", "100755")),
                    "selected path is not a regular file with directory ancestors")
        return mode, oid

    materials = []
    for expected in rights["current_source_material"]:
        path = expected["path"]
        safe_path(path)
        raw = regular(root / path, MAX_FILE_BYTES)
        mode, oid = locate(path)
        record = file_record(path, raw, mode)
        require(encode({k: record[k] for k in expected}) == encode(expected),
                "current selected source material changed")
        require(record["git_blob"] == oid, "selected worktree/Git membership changed")
        require(worktree_mode(root / path) == mode, "selected worktree/Git executable mode changed")
        materials.append(record)
    evidence = []
    for path, raw in evidence_raw.items():
        mode, oid = locate(path)
        record = file_record(path, raw, mode)
        require(record["git_blob"] == oid, "published evidence not in pinned source tree")
        require(worktree_mode(root / path) == mode, "published evidence executable mode changed")
        evidence.append(record)
    # Recheck every exported path after collection; neither bytes nor executable
    # mode may drift between the initial read and completion of this snapshot.
    for record in (*materials, *evidence):
        path = root / record["path"]
        require(regular(path, MAX_FILE_BYTES).hex() == record["content_hex"]
                and worktree_mode(path) == record["mode"], "selected source changed during export")
    value = {"format": FORMAT, "source_commit": expected_source_commit,
             "source_tree": expected_source_tree, "commit": object_record(expected_source_commit, commit),
             "trees": [tree_rows[oid] for oid in sorted(tree_rows)],
             "materials": sorted(materials, key=lambda row: row["path"]),
             "evidence": sorted(evidence, key=lambda row: row["path"]),
             "claim": CLAIM, "status": dict(STATUS)}
    raw = encode(value)
    inspect_package(raw, digest(raw), expected_source_commit=expected_source_commit,
                    expected_source_tree=expected_source_tree, expected_rights_sha256=expected_rights_sha256)
    return value


def write_new_or_equal(path, raw):
    """Create once; existing outputs are only read and compared, never overwritten."""
    path = Path(path).absolute()
    require(path.parent.resolve() == path.parent, "physical output directory required")
    if path.exists() or path.is_symlink():
        require(regular(path, MAX_PACKAGE_BYTES) == raw, "existing output differs; refusing overwrite")
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
    with os.fdopen(fd, "wb") as stream:
        stream.write(raw)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    commands = parser.add_subparsers(dest="command", required=True)
    required = commands.add_parser("required-pins", allow_abbrev=False)
    required.add_argument("--package", type=Path, required=True)
    required.add_argument("--expect-package-sha256", required=True)
    for action in ("export", "inspect"):
        sub = commands.add_parser(action, allow_abbrev=False)
        sub.add_argument("--expect-source-commit", required=True)
        sub.add_argument("--expect-source-tree", required=True)
        sub.add_argument("--expect-rights-sha256", required=True)
        if action == "export":
            sub.add_argument("--source-root", type=Path, required=True)
            sub.add_argument("--output", type=Path, required=True)
        else:
            sub.add_argument("--package", type=Path, required=True)
            sub.add_argument("--expect-package-sha256", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "required-pins":
            raw = regular(args.package, MAX_PACKAGE_BYTES)
            pin(args.expect_package_sha256, 64, "package SHA-256")
            require(digest(raw) == args.expect_package_sha256, "independent package SHA-256 mismatch")
            value = strict_json(raw)
            result = {"warning": "INFORMATION_ONLY; obtain source and rights pins independently before inspection",
                      "untrusted_package_claims": {k: value[k] for k in ("source_commit", "source_tree")},
                      "published_rights_sha256": RIGHTS_SHA256,
                      "required_independent_pins": ["package_sha256", "source_commit", "source_tree",
                                                    "published_rights_sha256"]}
        else:
            pins = {"expected_source_commit": args.expect_source_commit,
                    "expected_source_tree": args.expect_source_tree,
                    "expected_rights_sha256": args.expect_rights_sha256}
            if args.command == "export":
                raw = encode(build_package(args.source_root, **pins))
                write_new_or_equal(args.output, raw)
                result = {"output": str(args.output), "package_sha256": digest(raw), "bytes": len(raw),
                          "independent_pin_requirement": "Transfer this digest through an independent channel"}
            else:
                raw = regular(args.package, MAX_PACKAGE_BYTES)
                result = inspect_package(raw, args.expect_package_sha256, **pins)
        sys.stdout.buffer.write(encode(result))
    except (ValueError, OSError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        parser.exit(2, "source package refused: " + str(exc) + "\n")


if __name__ == "__main__":
    main()
