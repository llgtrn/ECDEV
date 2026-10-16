#!/usr/bin/env python3
"""Offline sealed delta comparison, optionally compiling an unchanged pure module with inert types."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys
import time

OLD_HELPER_SHA = "c92024d65f218a88eeae0644d1af0ac22fe64c1c3bb1f4787ce1f08f77882a91"
HANDOFF_PATH = "research/commerce/governance/price-eur-target-handoff-v1.json"
MAP_PATH = "research/commerce/governance/price-eur-target-case-map-v1.jsonl"
WITNESS_PATH = "research/commerce/governance/price-eur-target-membership-v1.json"
RESULTS_PATH = "research/commerce/governance/price-eur-pure-probe-v1.json"
PINS = {
    HANDOFF_PATH: "9657f611bee636dc08613eb3fe68511424afba764b474aed53fe1243c4a7fc20",
    MAP_PATH: "b8caf5500745de277cfb9d011722b05e884a0e8e76671e0e849756720f8cd16d",
    WITNESS_PATH: "2c1ca71af5a29d093ff2dbc1a325156a449c9bf1d8cf9ed2aa46f3c0891e675a",
    RESULTS_PATH: "fe3a4236412946e90cd2416fea3ca8db0ac2be175be4ec64297d7b7d5f8235b2",
}
HANDOFF_CANONICAL_SHA = "bab89a1b678642ad1dd03a601d71e1dc4add07faa70760b2520959ff4bdb3516"
WITNESS_CANONICAL_SHA = "ca91c0b9f90731f6f3b1a24c859922d2f83668ae0cd2146653a65dc343dc119a"
NEW_INDICES = [117, 212, 409, 855, 1188, 1193, 1849]
NEW_MANIFEST = "5beaf6ab248dde7a3c42e6ef1b177c64e8c3af481561e5dd2d76aa2d893d342e"
ALL_MANIFEST = "451f0bb1a2f3acd3879ddc2945c14e74447ec6ac70dfd5ebf374d086c8f353dc"


def regular(path, limit=16 * 1024 * 1024):
    path = Path(path).absolute()
    if ".." in path.parts or any(p.is_symlink() for p in (*path.parents, path)):
        raise ValueError("unsafe input path")
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError("nonregular or oversized input")
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("input exceeds byte budget")
    return data, stat.S_IMODE(info.st_mode)


def helpers():
    path = Path(__file__).absolute().with_name("price_handoff.py")
    before = regular(path)
    if hashlib.sha256(before[0]).hexdigest() != OLD_HELPER_SHA:
        raise ValueError("original portable comparison helper changed")
    old = {"__file__": str(path), "__name__": "sealed_previous_comparison"}
    exec(compile(before[0], str(path), "exec"), old)
    core = old["helpers"]()
    if regular(path) != before:
        raise ValueError("original helper changed during loading")
    return old, core


def sealed(value, expected, core, message):
    if core["sha256"](core["canonical"](value).encode()) != expected:
        raise ValueError(message)


def validate_witness(witness, handoff):
    _, core = helpers()
    sealed(handoff, HANDOFF_CANONICAL_SHA, core, "handoff identity, rights or claims changed")
    sealed(witness, WITNESS_CANONICAL_SHA, core, "membership witness changed")
    if core["git_oid"]("commit", witness["commit_preimage_utf8"].encode()) != witness["commit"]:
        raise ValueError("commit preimage mismatch")
    trees = {row["path"]: row for row in witness["tree_witness"]}
    if len(trees) != 9 or len(trees) != len(witness["tree_witness"]):
        raise ValueError("incomplete or duplicate compact trees")
    for directory, row in trees.items():
        entries = row["entries"]
        if len({e["name"] for e in entries}) != len(entries):
            raise ValueError("duplicate tree member")
        ordered = sorted(entries, key=lambda e: e["name"].encode() + (b"/" if e["type"] == "tree" else b""))
        body = b"".join(e["mode"].lstrip("0").encode() + b" " + e["name"].encode() + b"\0" + bytes.fromhex(e["oid"]) for e in ordered)
        if core["git_oid"]("tree", body) != row["oid"]:
            raise ValueError("compact tree hash mismatch")
    if trees[""]["oid"] != witness["tree"] or not witness["commit_preimage_utf8"].startswith("tree " + witness["tree"] + "\n"):
        raise ValueError("root tree does not bind commit")
    for file in witness["files"]:
        directory = ""
        parts = file["path"].split("/")
        for index, name in enumerate(parts):
            entry = next(e for e in trees[directory]["entries"] if e["name"] == name)
            if index == len(parts) - 1:
                if (entry["oid"], entry["mode"], entry["type"]) != (file["git_blob"], file["mode"], "blob"):
                    raise ValueError("blob membership mismatch")
            else:
                directory = "/".join(parts[:index + 1])
                if entry["type"] != "tree" or entry["oid"] != trees[directory]["oid"]:
                    raise ValueError("ancestor tree membership mismatch")
    return witness["files"]


def validate(corpus, oldresults, newresults, oldrows, newrows, handoff, witness):
    old, core = helpers()
    validate_witness(witness, handoff)
    original = core["build_proposal"](corpus, oldresults, core["portable_binding"]())
    if core["sha256"]((core["canonical"](oldresults) + "\n").encode()) != old["PINS"][old["RESULTS_PATH"]]:
        raise ValueError("original pure transcript changed")
    if not all(type(rows) is list and len(rows) == 2937 for rows in (oldrows, newrows, newresults)):
        raise ValueError("complete ordered corpus and outcome coverage required")
    mapping = handoff["source_case_mapping"]
    domains = [re.search(r"str\(([^)]+)\)", mapping[k]).group(1) for k in ("request_digest", "projection_digest")]
    supported, additions, changes, unmapped = [], [], [], []
    for index, (case, prior, result, previous, row) in enumerate(zip(corpus["cases"], oldresults, newresults, oldrows, newrows)):
        for outcome, published in ((prior, previous), (result, row)):
            if type(outcome) is not dict or type(outcome.get("index")) is not int or outcome["index"] != index:
                raise ValueError("wrong, duplicate or reordered result index")
            if type(published) is not dict or type(published.get("index")) is not int or published["index"] != index:
                raise ValueError("wrong, duplicate or reordered map index")
            keys = {"index", "request_sha256", "source_projection_sha256", "native"}
            if published.get("request_sha256") != old["request_digest"](case, domains[0]) or published.get("source_projection_sha256") != old["projection_digest"](case["expected"], domains[1]):
                raise ValueError("request or source projection digest mismatch")
            if outcome.get("status") == "OK":
                expected = {k: case["expected"][k] for k in core["FIELDS"][:-1]}
                number = case["expected"]["amount_float"]
                expected["amount_float_bits"] = None if number is None else struct.pack(">d", number).hex()
                if (set(outcome) != {"index", "status", *core["FIELDS"]} or case["error"] is not None
                        or core["canonical"]({k: outcome[k] for k in core["FIELDS"]}) != core["canonical"](expected)
                        or set(published) != keys | {"result_sha256"} or published["native"] != "SUPPORTED"
                        or published["result_sha256"] != old["projection_digest"](outcome, domains[1])
                        or published["result_sha256"] != published["source_projection_sha256"]):
                    raise ValueError("four-field, IEEE bits, source error or supported map mismatch")
            elif (set(outcome) != {"index", "status", "code"} or outcome["status"] != "ERR"
                  or type(outcome["code"]) is not int or not 1 <= outcome["code"] <= 16
                  or set(published) != keys | {"unavailable"} or published["native"] != "UNMAPPED"
                  or type(published["unavailable"]) is not int or published["unavailable"] != outcome["code"]):
                raise ValueError("typed unavailability mismatch")
        if prior["status"] == "OK" and (core["canonical"](prior) != core["canonical"](result) or previous != row):
            raise ValueError("original supported outcome rebound or changed")
        if result["status"] == "OK":
            supported.append(index)
            if prior["status"] == "ERR":
                additions.append(index)
        else:
            unmapped.append({"index": index, "code": result["code"], "target": None})
            if prior != result:
                changes.append({"index": index, "from": prior["code"], "to": result["code"]})
    def manifest(indices):
        return core["sha256"]("".join(f"{i}\t{corpus['cases'][i]['name']}\n" for i in indices).encode())
    if additions != NEW_INDICES or manifest(additions) != NEW_MANIFEST or len(supported) != 106 or len(unmapped) != 2831 or manifest(supported) != ALL_MANIFEST:
        raise ValueError("qualified subset differs from exact delta")
    if (changes != mapping["reason_only_EUR_changes"] or len(changes) != 27
            or any(r["from"] != 5 or r["to"] != (11 if r["index"] == 1253 else 8) for r in changes)
            or supported != mapping["supported_indices"] or additions != mapping["new_supported_indices"]):
        raise ValueError("refusal delta differs from exact 27 cases")
    if core["sha256"]((core["canonical"](newresults) + "\n").encode()) != PINS[RESULTS_PATH]:
        raise ValueError("new actual pure transcript changed")
    candidates = []
    for index in additions:
        case, result = corpus["cases"][index], newresults[index]
        request = {k: case[k] for k in core["SELECTORS"]}
        bits = result["amount_float_bits"]
        candidates.append({"index": index, "name": case["name"], "corpus_locator": f"/cases/{index}", "request": request,
                           "request_presence": {k: v is not None for k, v in request.items()},
                           "derived": {**{k: result[k] for k in core["FIELDS"]}, "amount_float": None if bits is None else struct.unpack(">d", bytes.fromhex(bits))[0]},
                           "target": handoff["target_binding"], "status": "CANDIDATE_PENDING_RIGHTS_AND_ADMISSION", "canonical_assignment": None})
    return {"format": "ecdev-price-eur-reconciliation-v1", "scope": "SEALED_SOURCE_ONLY_DELTA; NO_AUTHORITY_OR_GOVERNED_EXECUTION",
            "total": 2937, "supported": 106, "unmapped": 2831, "new_candidate_count": 7, "new_candidates": candidates,
            "new_manifest_sha256": NEW_MANIFEST, "all106_manifest_sha256": ALL_MANIFEST, "qualified_case_indices": supported,
            "unmapped_cases": unmapped, "reason_only_changes": changes,
            "old99": {"count": 99, "status": "UNCHANGED_PRIOR_QUALIFICATION; NOT_REBOUND", "manifest_sha256": core["SUBSET"], "implementation_key": handoff["historical_baseline"]["implementation_key"]},
            "source_ids": original["source"]["source_ids"], "original_819_assignments_changed": 0,
            "target_publication": {"commit": witness["commit"], "tree": witness["tree"], "retained_handoff_fields": handoff["publication"]},
            "canonical_assignment": None, "accepted_owner": None, "authority_grants": [], "copying_authorization": "UNKNOWN",
            "admission": "PENDING", "source_adapter_cutover": False, "fallback": False, "governed_caller_locally_run": False}


def load_inputs(root):
    old, core = helpers()
    root = core["physical_path"](root)
    corpus, _ = core["load_source"](root)
    raw, snapshot = {}, {}
    for relative, digest in {**core["SOURCE_PINS"], **old["PINS"], **PINS}.items():
        path = root / relative
        data, mode = regular(path)
        if core["sha256"](data) != digest:
            raise ValueError(f"sealed comparison input changed: {relative}")
        raw[relative] = data
        snapshot[str(path)] = (digest, mode)
    for name in ("price_subset.py", "price_handoff.py", Path(__file__).name):
        path = Path(__file__).absolute().with_name(name)
        data, mode = regular(path)
        snapshot[str(path)] = (core["sha256"](data), mode)
    def parsed(path):
        return core["parse_json"](raw[path])
    def rows(path):
        return [core["parse_json"](line) for line in raw[path].splitlines()]
    args = (corpus, parsed(old["RESULTS_PATH"]), parsed(RESULTS_PATH), rows(old["MAP_PATH"]), rows(MAP_PATH), parsed(HANDOFF_PATH), parsed(WITNESS_PATH))
    old["validate"](corpus, args[1], parsed(old["PROPOSAL_PATH"]), parsed(old["HANDOFF_PATH"]), args[3])
    for path in (HANDOFF_PATH, MAP_PATH):
        member = next(f for f in args[-1]["files"] if f["sha256"] == PINS[path])
        if len(raw[path]) != member["bytes"] or core["git_oid"]("blob", raw[path]) != member["git_blob"]:
            raise ValueError("published blob body mismatch")
    return args, snapshot


def stable(snapshot, core):
    for path, expected in snapshot.items():
        data, mode = regular(path, 256 * 1024 * 1024)
        if (core["sha256"](data), mode) != expected:
            raise ValueError("qualification input bytes or mode changed during execution")


def reconcile(root=None):
    old, core = helpers()
    root = Path(__file__).absolute().parents[2] if root is None else root
    args, snapshot = load_inputs(root)
    result = validate(*args)
    stable(snapshot, core)
    return {**result, "input_sha256": {**core["SOURCE_PINS"], **old["PINS"], **PINS},
            "helper_sha256": {"prior_comparison": OLD_HELPER_SHA, "pure_module": old["CORE_SHA"]},
            "blob_bodies_locally_rehashed": 2, "parser_body": "SEALED_TREE_MEMBER_METADATA_ONLY; NOT_LOADED"}


def run(reference, output, root=None, rustc=None):
    _, core = helpers()
    root = core["physical_path"](Path(__file__).absolute().parents[2] if root is None else root)
    reference, output = (core["physical_path"](p) for p in (reference, output))
    compiler = core["physical_path"](rustc or shutil.which("rustc") or "")
    core["separate_output"](output, [root, Path(__file__).absolute().parents[2], reference.parent, compiler])
    args, snapshot = load_inputs(root)
    checked = validate(*args)
    source, source_mode = regular(reference)
    member = next(f for f in args[-1]["files"] if f["path"] == args[-2]["target_binding"]["artifact"]["path"])
    if len(source) != member["bytes"] or core["sha256"](source) != member["sha256"] or core["git_oid"]("blob", source) != member["git_blob"] or bool(source_mode & 0o111) != (member["mode"] == "100755"):
        raise ValueError("reference differs from pinned pure module")
    compiler_bytes, compiler_mode = regular(compiler, 256 * 1024 * 1024)
    if compiler.name == "rustup" or not compiler_mode & 0o111:
        raise ValueError("provide a direct installed compiler executable")
    snapshot.update({str(reference): (core["sha256"](source), source_mode), str(compiler): (core["sha256"](compiler_bytes), compiler_mode)})
    output.mkdir()
    for name in ("home", "tmp"):
        (output / name).mkdir()
    for name, data in (("price_text.rs", source), ("probe.rs", core["WRAPPER"].encode()), ("stdin.tsv", core["encode_requests"](args[0]))):
        path = output / name
        path.write_bytes(data)
        written, mode = regular(path)
        if written != data:
            raise ValueError("generated probe input changed while writing")
        snapshot[str(path)] = (core["sha256"](written), mode)
    env = {"PATH": str(compiler.parent) + ":/usr/bin:/bin", "HOME": str(output / "home"), "TMPDIR": str(output / "tmp"), "LANG": "C", "LC_ALL": "C", "TZ": "UTC"}
    commands = []
    def execute(command, stdin=None):
        commands.append([str(a) for a in command])
        prefix = output / f"command-{len(commands):02}"
        with open(prefix.with_suffix(".stdout"), "wb") as stdout, open(prefix.with_suffix(".stderr"), "wb") as stderr:
            try:
                process = subprocess.Popen(command, cwd=output, env=env, stdin=stdin or subprocess.DEVNULL, stdout=stdout, stderr=stderr)
            except OSError as error:
                stderr.write(str(error).encode())
                raise ValueError(f"command launch failed; logs at {prefix}: {error}") from error
            deadline = time.monotonic() + 60
            while process.poll() is None:
                if time.monotonic() > deadline or os.fstat(stdout.fileno()).st_size + os.fstat(stderr.fileno()).st_size > 4 * 1024 * 1024:
                    process.kill()
                    process.wait()
                    raise ValueError(f"command timeout or output limit; logs at {prefix}")
                time.sleep(0.01)
        out, err = (regular(prefix.with_suffix(suffix), 4 * 1024 * 1024)[0] for suffix in (".stdout", ".stderr"))
        if len(out) + len(err) > 4 * 1024 * 1024 or process.returncode:
            raise ValueError(f"command failed ({process.returncode}); logs at {prefix}: " + err[-4096:].decode(errors="replace"))
        return out
    version = execute([str(compiler), "--version", "--verbose"]).decode()
    execute([str(compiler), "--edition=2021", "-Awarnings", str(output / "probe.rs"), "-o", str(output / "probe")])
    binary, mode = regular(output / "probe")
    snapshot[str(output / "probe")] = (core["sha256"](binary), mode)
    with open(output / "stdin.tsv", "rb") as stdin:
        stdout = execute([str(output / "probe")], stdin)
    actual = core["parse_results"](stdout)
    checked = validate(args[0], args[1], actual, *args[3:])
    stable(snapshot, core)
    result_bytes = (core["canonical"](actual) + "\n").encode()
    (output / "results.json").write_bytes(result_bytes)
    (output / "reconciliation.json").write_text(core["canonical"](checked) + "\n")
    receipt = {"format": "ecdev-price-eur-pure-qualification-v1", "status": "PASS", "scope": "PURE_MODULE_WITH_INERT_TYPE_STUBS_ONLY",
               "compiler": {"path": str(compiler), "sha256": core["sha256"](compiler_bytes), "mode": compiler_mode, "version": version},
               "commands": commands, "wrapper_sha256": core["sha256"](core["WRAPPER"].encode()), "parser_sha256": member["sha256"],
               "binary_sha256": core["sha256"](regular(output / "probe")[0]), "stdin_sha256": core["sha256"](regular(output / "stdin.tsv")[0]),
               "stdout_sha256": core["sha256"](stdout), "results_sha256": core["sha256"](result_bytes),
               "target_publication": checked["target_publication"], "parser_git_blob": member["git_blob"],
               "new_candidate_indices": NEW_INDICES, "new_manifest_sha256": NEW_MANIFEST, "all106_manifest_sha256": ALL_MANIFEST,
               "input_files": [{"path": p, "sha256": digest, "mode": mode} for p, (digest, mode) in sorted(snapshot.items())],
               "stability_scope": "ENUMERATED_FILE_BYTES_AND_MODES_ONLY; SYSROOT_LINKER_DEPENDENCIES_NOT_ATTESTED", "blob_bodies_locally_rehashed": 3,
               "supported": 106, "unmapped": 2831, "new_candidates": 7, "governed_caller_locally_run": False,
               "canonical_assignment": None, "accepted_owner": None, "authority_grants": [], "copying_authorization": "UNKNOWN", "admission": "PENDING"}
    (output / "qualification.json").write_text(core["canonical"](receipt) + "\n")
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).absolute().parents[2])
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--rustc", type=Path)
    args = parser.parse_args()
    try:
        if bool(args.reference) != bool(args.output) or (args.rustc and not args.reference):
            raise ValueError("reference and new output are required together; compiler requires reference")
        result = run(args.reference, args.output, args.source_root, args.rustc) if args.reference else reconcile(args.source_root)
        print(helpers()[1]["canonical"](result))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError, StopIteration, subprocess.SubprocessError) as error:
        print(f"qualification refused: {error}", file=sys.stderr)
        sys.exit(2)
