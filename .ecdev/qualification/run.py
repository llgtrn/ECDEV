#!/usr/bin/env python3
"""Qualify a pinned, supplied central reader without changing either source tree.

This tool fetches nothing. It checks the entire engine archive against immutable
local Git objects before compiling the interoperability harness in a new scratch
directory. Successful intake grants no source registry, rights or acceptance credit.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys


def git(objects, *args):
    env = {key: os.environ[key] for key in ("PATH", "HOME") if key in os.environ}
    env.update(GIT_NO_REPLACE_OBJECTS="1", GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0",
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull, GIT_OPTIONAL_LOCKS="0")
    return subprocess.run(
        ["git", "--git-dir", str(objects), "-c", "protocol.allow=never", "-c", "core.fsmonitor=false", *args], env=env,
        check=True, capture_output=True,
    ).stdout


def verify_reference(reference, objects, contract):
    """Check every tree entry, including tests, before executing supplied code."""
    commit = contract["engine_commit"]
    tree = git(objects, "rev-parse", f"{commit}^{{tree}}").decode().strip()
    if tree != contract["engine_tree"]:
        raise ValueError("pinned engine commit/tree mismatch")
    entries = git(objects, "ls-tree", "-r", "-z", "--full-tree", commit)
    expected = {}
    for row in entries.split(b"\0"):
        if not row:
            continue
        header, raw_path = row.split(b"\t", 1)
        mode, kind, oid = header.decode("ascii").split()
        path = raw_path.decode("utf-8")
        if mode not in ("100644", "100755") or kind != "blob":
            raise ValueError(f"unsupported engine tree entry: {path}")
        if path.startswith("/") or any(p in ("", ".", "..") for p in path.split("/")):
            raise ValueError("unsafe engine tree path")
        expected[path] = (mode, oid)
    actual = {}
    total = 0
    for base, directories, files in os.walk(reference, followlinks=False):
        for name in directories:
            if Path(base, name).is_symlink():
                raise ValueError("engine reference contains a directory symlink")
        for name in files:
            path = Path(base, name)
            relative = path.relative_to(reference).as_posix()
            metadata = path.lstat()
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 64 * 1024 * 1024:
                raise ValueError(f"nonregular or oversized engine input: {relative}")
            total += metadata.st_size
            if total > 512 * 1024 * 1024 or len(actual) >= 100_000:
                raise ValueError("engine reference exceeds qualification budget")
            data = path.read_bytes()
            blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
            actual[relative] = ("100755" if metadata.st_mode & 0o111 else "100644", blob)
    if actual != expected:
        missing = sorted(expected.keys() - actual.keys())
        extra = sorted(actual.keys() - expected.keys())
        changed = sorted(p for p in actual.keys() & expected.keys() if actual[p] != expected[p])
        raise ValueError(f"engine reference differs from pin: missing={missing[:5]}, extra={extra[:5]}, changed={changed[:5]}")
    return {"tree": tree, "files": len(actual), "bytes": total, "status": "ALL_PINNED_BLOBS_AND_EXECUTABLE_MODES_MATCH"}


def separate_output(output, protected):
    for root in protected:
        if output == root or output.is_relative_to(root) or root.is_relative_to(output):
            raise ValueError("output must be disjoint from source, engine reference and object store")
    if output.exists() or output.is_symlink():
        raise ValueError("output directory must be new; existing evidence is never overwritten")


def physical_path(path):
    if ".." in path.parts:
        raise ValueError("qualification paths must not contain parent traversal")
    absolute = path.absolute()
    for component in (*reversed(absolute.parents), absolute):
        if component.is_symlink():
            raise ValueError(f"qualification path contains a symlink: {component}")
    return absolute.resolve()


def source_binding(source, supplemental):
    paths = set(git(source / ".git", "ls-files", "--cached", "--full-name", "-z").decode().split("\0")) - {""}
    indexed = sorted(paths)
    paths.update(supplemental)
    rows = []
    for relative in sorted(paths):
        path = source / relative
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"source input is missing or not regular: {relative}")
        data = path.read_bytes()
        rows.append({"path": relative, "sha256": hashlib.sha256(data).hexdigest(),
                     "bytes": len(data), "executable": bool(path.stat().st_mode & 0o111)})
    manifest = json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()
    return {"observed_head": git(source / ".git", "rev-parse", "HEAD^{commit}").decode().strip(),
            "manifest_sha256": hashlib.sha256(manifest).hexdigest(), "inputs": rows,
            "index_paths": indexed,
            "scope": "INDEX_SELECTED_WORKTREE_BYTES_PLUS_EXPLICIT_QUALIFICATION_INPUTS; COMMIT_MEMBERSHIP_NOT_ATTESTED"}


def build_environment(project, output):
    # Cargo reads configuration from its working directory's ancestors and its
    # cache home. Refuse those overrides instead of executing an unbound wrapper,
    # registry patch, linker or build target. No credentials enter the child env.
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    for directory in (*project.parents, project, cargo_home):
        for relative in (".cargo/config", ".cargo/config.toml") if directory != cargo_home else ("config", "config.toml"):
            if (directory / relative).exists():
                raise ValueError(f"unbound Cargo configuration: {directory / relative}")
    env = {key: os.environ[key] for key in ("PATH", "HOME", "CARGO_HOME", "RUSTUP_HOME") if key in os.environ}
    env.update(CARGO_TARGET_DIR=str(output / "target"), CARGO_INCREMENTAL="0",
               CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0",
               CARGO_NET_OFFLINE="true", RUSTUP_AUTO_INSTALL="0",
               TMPDIR=str(output / "tmp"), LANG="C", LC_ALL="C", TZ="UTC",
               GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
               GIT_CONFIG_SYSTEM=os.devnull, GIT_NO_REPLACE_OBJECTS="1",
               GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0", GIT_OPTIONAL_LOCKS="0",
               GIT_CONFIG_COUNT="2", GIT_CONFIG_KEY_0="protocol.allow", GIT_CONFIG_VALUE_0="never",
               GIT_CONFIG_KEY_1="core.fsmonitor", GIT_CONFIG_VALUE_1="false")
    return env


def validate_report(report, contract):
    required = {
        "format": "ecdev-central-intake-qualification-v1", "status": "PASS",
        "engine_commit": contract["engine_commit"],
        "protocol_version": contract["protocol_version"],
        "compatibility_epoch": contract["compatibility_epoch"],
        "central_schema": contract["schema_digest"],
        "central_engine": contract["engine_digest"],
        "registered_sources": 7, "source_registered": False, "tests_failed": 0,
    }
    if not isinstance(report, dict) or any(type(report.get(k)) is not type(v) or report[k] != v for k, v in required.items()):
        raise ValueError("harness result does not match the reviewed reader contract")
    for lane in ("fixture", "fresh"):
        if report.get(lane, {}).get("registry_binding") != "PENDING":
            raise ValueError("intake result cannot grant canonical registry binding")
    if report["fixture"].get("sha256") != "sha256:" + contract["fixture_sha256"]:
        raise ValueError("harness fixture result differs from immutable fixture pin")
    checks = report.get("checks", []) + report.get("reader_probes", [])
    if not checks or any(row.get("pass") is not True for row in checks) or report.get("tests_passed") != len(checks):
        raise ValueError("harness checks are incomplete or failed")


def run(reference, objects, output, cargo, toolchain):
    source = Path(__file__).resolve().parents[2]
    contract_path = source / "research/commerce/governance/central-reader-contract-v1.json"
    contract_bytes = contract_path.read_bytes()
    contract = json.loads(contract_bytes)
    if contract["format"] != "ecdev-central-reader-contract-v1":
        raise ValueError("unsupported qualification contract")
    reference, objects, output = (physical_path(path) for path in (reference, objects, output))
    separate_output(output, (source, reference, objects))
    binding = verify_reference(reference, objects, contract)
    source_inputs = source_binding(source, (
        str(contract_path.relative_to(source)), ".ecdev/qualification/run.py",
        ".ecdev/qualification/central_intake.rs", ".ecdev/qualification/test_run.py",
    ))
    output.mkdir(parents=False)
    project = output / "harness"
    (project / "src").mkdir(parents=True)
    # JSON strings are valid quoted TOML strings for these path values.
    manifest = (
        '[package]\nname = "ecdev-central-reader-qualification"\nversion = "0.0.0"\nedition = "2021"\n'
        '[workspace]\n[dependencies]\n'
        f'ecdev_governance = {{ package = "ecdev-governance", path = {json.dumps(str(source / ".ecdev"))} }}\n'
        f'ynventa = {{ path = {json.dumps(str(reference / "apps/cli"))} }}\n'
    )
    (project / "Cargo.toml").write_text(manifest)
    harness = (source / ".ecdev/qualification/central_intake.rs").read_bytes()
    (project / "src/main.rs").write_bytes(harness)
    (output / "tmp").mkdir()
    env = build_environment(project, output)
    commands = [
        [cargo, f"+{toolchain}", "generate-lockfile", "--offline", "--manifest-path", str(project / "Cargo.toml")],
        [cargo, f"+{toolchain}", "clippy", "--offline", "--frozen", "--all-targets", "--manifest-path", str(project / "Cargo.toml"), "--", "-D", "warnings"],
        [cargo, f"+{toolchain}", "run", "--offline", "--frozen", "--manifest-path", str(project / "Cargo.toml"), "--", str(source), str(output)],
    ]
    with (output / "cargo.log").open("wb") as log:
        for command in commands:
            result = subprocess.run(command, env=env, cwd=project, stdout=subprocess.PIPE, stderr=log, check=False)
            if result.returncode:
                (output / "failed-command-stdout").write_bytes(result.stdout)
                raise ValueError(f"qualification command failed ({result.returncode}); see {output / 'cargo.log'}")
    (output / "unvalidated-harness-output.json").write_bytes(result.stdout)
    report = json.loads(result.stdout)
    validate_report(report, contract)
    # Recheck the full reference after compilation and execution. This is byte/mode
    # stability under the existing trusted-filesystem model, not hostile-race attestation.
    if verify_reference(reference, objects, contract) != binding:
        raise ValueError("engine input binding changed during qualification")
    if source_binding(source, (row["path"] for row in source_inputs["inputs"])) != source_inputs:
        raise ValueError("source input binding changed during qualification")
    if contract_path.read_bytes() != contract_bytes:
        raise ValueError("qualification contract changed during qualification")
    report["engine_input_binding"] = binding
    report["source_input_binding"] = source_inputs
    report["contract_sha256"] = hashlib.sha256(contract_bytes).hexdigest()
    report["harness_sha256"] = hashlib.sha256(harness).hexdigest()
    report["runner_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    report["binary_sha256"] = hashlib.sha256((output / "target/debug/ecdev-central-reader-qualification").read_bytes()).hexdigest()
    report["cargo_lock_sha256"] = hashlib.sha256((project / "Cargo.lock").read_bytes()).hexdigest()
    compiler = Path(shutil.which(cargo) or cargo).parent / "rustc"
    report["compiler"] = subprocess.run([str(compiler), f"+{toolchain}", "-vV"], env=env,
                                       cwd=project, check=True, capture_output=True, text=True).stdout
    report["build_environment"] = env
    report["qualification_contract"] = contract
    report["execution_commands"] = commands
    report["source_lock_installed"] = False
    report["binding_trust"] = "LOCAL_IMMUTABLE_GIT_OBJECTS_AND_TRUSTED_FILESYSTEM; NOT_AUTHENTICATED_EXECUTION"
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(output / "report.json")


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
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"qualification refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
