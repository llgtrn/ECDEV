#!/usr/bin/env python3
"""Offline pure-parser qualification. No governed caller, rights or admission is executed."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import struct
import subprocess
import sys

COMMIT = "c312915450d6a717f9144ecdc22718afbce3dd9d"
TREE = "5dc0ed13ad1602bd1894ddc0904cb9f583176ce9"
PARENT = "03689d420e03fe06b7fecc30eb4d3673e67d6354"
SUBSET = "d5794485dd05bcb51e79fcc5af2a0ea6a1a915800a829ee0211cd3df985df29f"
CORPUS = "adapter/web/tests/fixtures/price-contract.json"
HANDOFF = "research/commerce/governance/price-public-api-source-handoff-v1.json"
MAPPING = "research/commerce/governance/source-mapping-proposal-v1.json"
METADATA = "research/commerce/governance/price-text-target-custody-v1.json"
CUSTODY_PATH = METADATA
CUSTODY_SHA = "b8fc4a80c4efb9def85c298b53172a0c5bf7b2ec817308158cdb2b2d0e110350"
CAPTURE = ".ecdev/tests/fixtures/source-capture-00dd7f0-v1.json"
SOURCE_PINS = {
    CORPUS: "3729e24b33e660f6155ccca4ef5e8b199de4b2cab1e7b517cf403f930157d3ce",
    HANDOFF: "8915a6b55a2eceedfd2942e494330d159158a56f4a4818dfada3af8a556b7a0f",
    MAPPING: "f2933fd99d4e2737dd3abb4233d7d8c634409a09597c2a01783016930745b056",
    CAPTURE: "ef930f0e26aab701ef384f9992d0b32abf3d101ac5bcb842ffa4d045bdce9a1d",
    METADATA: CUSTODY_SHA,
}
SELECTORS = ("input", "currency_hint", "decimal_separator", "digit_group_separator")
FIELDS = ("amount", "currency", "amount_text", "amount_float_bits")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def parse_json(data):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate decoded JSON key")
            result[key] = value
        return result
    return json.loads(data, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite JSON")))


def physical_path(path):
    path = Path(path)
    if ".." in path.parts:
        raise ValueError("parent traversal in qualification path")
    path = path.absolute()
    for component in (*reversed(path.parents), path):
        if component.is_symlink():
            raise ValueError("symlink in qualification path")
    return path.resolve()


def read_regular(path, limit=16 * 1024 * 1024):
    path = physical_path(path)
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as stream:
        meta = os.fstat(stream.fileno())
        if not stat.S_ISREG(meta.st_mode) or meta.st_size > limit:
            raise ValueError("nonregular or oversized qualification input")
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("qualification input exceeds byte budget")
    return data, bool(meta.st_mode & 0o111)


def git_oid(kind, data):
    if kind not in ("blob", "tree", "commit") or type(data) is not bytes:
        raise ValueError("invalid Git object")
    return hashlib.sha1(kind.encode() + b" " + str(len(data)).encode() + b"\0" + data).hexdigest()


def custody():
    data, _ = read_regular(Path(__file__).resolve().parents[2] / CUSTODY_PATH)
    if sha256(data) != CUSTODY_SHA:
        raise ValueError("sealed target custody artifact changed")
    return parse_json(data)


def verify_git_tree(tree):
    """Reconstruct every immediate directory of a complete recursive Git tree response."""
    if (type(tree) is not dict or tree.get("truncated") is not False
            or type(tree.get("tree")) is not list or len(tree["tree"]) > 100_000):
        raise ValueError("complete nontruncated Git tree required")
    members = {}
    for entry in tree["tree"]:
        if type(entry) is not dict:
            raise ValueError("invalid Git tree entry")
        path = entry.get("path")
        if (type(path) is not str or not path or "\\" in path or "\0" in path
                or path.startswith("/") or any(p in ("", ".", "..") for p in path.split("/"))):
            raise ValueError("unsafe Git tree path")
        mode_kind = (entry.get("mode"), entry.get("type"))
        if mode_kind not in (("040000", "tree"), ("100644", "blob"), ("100755", "blob"), ("120000", "blob"), ("160000", "commit")):
            raise ValueError("invalid Git tree mode/type")
        if path in members or type(entry.get("sha")) is not str or re.fullmatch("[0-9a-f]{40}", entry["sha"]) is None:
            raise ValueError("duplicate Git tree path or invalid object id")
        members[path] = entry
    children = {"": []}
    for path, entry in members.items():
        parent, _, name = path.rpartition("/")
        if parent and (parent not in members or members[parent]["type"] != "tree"):
            raise ValueError("Git tree ancestor is missing or not a directory")
        children.setdefault(parent, []).append((name, entry))
        if entry["type"] == "tree":
            children.setdefault(path, [])
    computed = {}
    for directory in sorted(children, key=lambda p: (p.count("/") + bool(p), p), reverse=True):
        body = b""
        for name, entry in sorted(children[directory], key=lambda item: item[0].encode() + (b"/" if item[1]["type"] == "tree" else b"")):
            oid = computed[f"{directory}/{name}".lstrip("/")] if entry["type"] == "tree" else entry["sha"]
            body += entry["mode"].lstrip("0").encode() + b" " + name.encode() + b"\0" + bytes.fromhex(oid)
        computed[directory] = git_oid("tree", body)
        if directory and computed[directory] != members[directory]["sha"]:
            raise ValueError("Git subtree content address mismatch")
    if computed[""] != tree.get("sha"):
        raise ValueError("Git root tree content address mismatch")
    return computed[""], members


def verify_tree(tree):
    oid, members = verify_git_tree(tree)
    if oid != TREE or len(members) != 4211:
        raise ValueError("target tree differs from exact pin")
    return members


def custody_to_binding(metadata):
    if canonical(metadata) != canonical(custody()):
        raise ValueError("target metadata differs from sealed artifact")
    return {"commit": metadata["commit"], "tree": metadata["tree"], "metadata_sha256": CUSTODY_SHA,
            "files": [{key: row[key] for key in ("path", "git_blob", "sha256", "bytes", "mode")}
                      for row in sorted(metadata["files"], key=lambda row: row["path"])]}


def portable_binding():
    return custody_to_binding(custody())


def load_reference(reference):
    reference = physical_path(reference)
    binding = portable_binding()
    files_expected = {row["path"]: row for row in binding["files"]}
    raw = {}
    for base, directories, files in os.walk(reference, followlinks=False):
        if any(Path(base, name).is_symlink() for name in directories):
            raise ValueError("target directory symlink")
        for name in files:
            path = Path(base, name)
            relative = path.relative_to(reference).as_posix()
            if relative not in set(files_expected) | {"commit-response.json", "tree-response.json"}:
                raise ValueError("unexpected target reference file")
            raw[relative] = read_regular(path)
    if set(raw) != set(files_expected) | {"commit-response.json", "tree-response.json"}:
        raise ValueError("incomplete target reference")
    commit = parse_json(raw["commit-response.json"][0])
    if (type(commit) is not dict or commit.get("sha") != COMMIT
            or type(commit.get("tree")) is not dict or commit["tree"].get("sha") != TREE
            or type(commit.get("parents")) is not list or len(commit["parents"]) != 1
            or type(commit["parents"][0]) is not dict or commit["parents"][0].get("sha") != PARENT
            or type(commit.get("message")) is not str):
        raise ValueError("target commit metadata mismatch")
    headers = [f"tree {TREE}", f"parent {PARENT}"]
    for role in ("author", "committer"):
        identity = commit.get(role)
        if (type(identity) is not dict or any(type(identity.get(key)) is not str for key in ("name", "email", "date"))
                or any(any(c in identity[key] for c in "\n\r<>\0") for key in ("name", "email"))):
            raise ValueError("invalid commit identity framing")
        at = int(datetime.strptime(identity["date"], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc).timestamp())
        headers.append(f"{role} {identity['name']} <{identity['email']}> {at} +0000")
    preimage = ("\n".join(headers) + "\n\n" + commit["message"] + "\n").encode()
    verification = commit.get("verification")
    if (git_oid("commit", preimage) != COMMIT or type(verification) is not dict
            or verification.get("reason") != "unsigned" or verification.get("verified") is not False
            or verification.get("signature") is not None or verification.get("payload") is not None):
        raise ValueError("unsigned target commit cannot be reconstructed")
    members = verify_tree(parse_json(raw["tree-response.json"][0]))
    for path, row in files_expected.items():
        blob, digest, size, mode = (row[key] for key in ("git_blob", "sha256", "bytes", "mode"))
        data, executable = raw[path]
        if path not in members or (members[path]["sha"], members[path]["mode"], members[path].get("size")) != (blob, mode, size):
            raise ValueError("target blob membership mismatch")
        if executable != (mode == "100755") or len(data) != size or git_oid("blob", data) != blob or sha256(data) != digest:
            raise ValueError("target source bytes or executable mode mismatch")
    parser_paths = [path for path in files_expected if path.endswith("/src/price_text.rs")]
    if len(parser_paths) != 1:
        raise ValueError("ambiguous sealed parser entry")
    return {**binding, "source_code": raw[parser_paths[0]][0],
            "snapshot": {str(reference / p): (sha256(b), x) for p, (b, x) in raw.items()}}


def load_source(source):
    raw = {}
    for path, expected in SOURCE_PINS.items():
        data, mode = read_regular(source / path)
        if sha256(data) != expected:
            raise ValueError(f"source pin mismatch: {path}")
        raw[path] = (data, mode)
    mapping = parse_json(raw[MAPPING][0])
    rows = mapping["node_mappings"]
    if len(rows) != 819 or len({r["source_id"] for r in rows}) != 819 or any(r["proposed_target"] is not None for r in rows):
        raise ValueError("original 819 assignments must remain null and unchanged")
    return parse_json(raw[CORPUS][0]), {str(source / p): (sha256(b), mode) for p, (b, mode) in raw.items()}


def build_proposal(corpus, results, binding):
    if type(binding) is not dict or set(binding) - set(portable_binding()) - {"source_code", "snapshot"}:
        raise ValueError("unknown target binding fields")
    if canonical({k: binding.get(k) for k in portable_binding()}) != canonical(portable_binding()):
        raise ValueError("target candidate custody mismatch")
    if sha256(canonical(corpus).encode()) != "93fdb960629e3b3b0181a8064485b06805e0e31af2e974efca01a62e455f3450":
        raise ValueError("parsed source corpus differs from immutable pin")
    if corpus.get("commit_sha") != "64e213a46a40473ba4f8aa3b249917fdc64d8a16" or len(corpus.get("cases", [])) != 2937 or len(results) != 2937:
        raise ValueError("complete pinned source corpus/outcomes required")
    metadata = custody()
    declaration = metadata["declared_target"]
    target = {"repository": metadata["target_repository"], "commit": COMMIT, "tree": TREE,
              "capability": declaration["capability"], "kernel": declaration["implementation_name"],
              "implementation_key": metadata["exact_implementation_key"],
              "declared_owner": declaration["owner"], "declared_caller": declaration["caller"]}
    mapped, unmapped, manifest = [], [], ""
    for index, (case, result) in enumerate(zip(corpus["cases"], results)):
        if type(result) is not dict or type(result.get("index")) is not int or result["index"] != index:
            raise ValueError("duplicate, reordered or malformed probe index")
        row = {"index": index, "name": case["name"], "corpus_locator": f"/cases/{index}"}
        if result.get("status") == "ERR":
            if set(result) != {"index", "status", "code"} or type(result["code"]) is not int or not 1 <= result["code"] <= 16:
                raise ValueError("invalid typed target unavailability")
            unmapped.append({**row, "code": result["code"], "target": None, "status": "UNMAPPED"})
            continue
        if result.get("status") != "OK" or set(result) != {"index", "status", *FIELDS}:
            raise ValueError("invalid probe success shape")
        expected = case["expected"]
        number = expected["amount_float"]
        expected_fields = {key: expected[key] for key in FIELDS[:-1]}
        expected_fields["amount_float_bits"] = None if number is None else struct.pack(">d", number).hex()
        if case["error"] is not None or canonical({k: result[k] for k in FIELDS}) != canonical(expected_fields):
            raise ValueError("source error or four-field/IEEE mismatch")
        request = {key: case[key] for key in SELECTORS}
        if any(value is not None and type(value) is not str for value in request.values()):
            raise ValueError("wrong-typed source selector")
        manifest += f"{index}\t{case['name']}\n"
        mapped.append({**row, "status": "CANDIDATE_PENDING_RIGHTS_AND_ADMISSION", "target": target,
                       "canonical_assignment": None, "request": request,
                       "request_presence": {key: value is not None for key, value in request.items()},
                       "payload_sha256": sha256((request["input"] or "").encode()),
                       "derived": {**{key: result[key] for key in FIELDS},
                                   "amount_float": None if result["amount_float_bits"] is None else struct.unpack(">d", bytes.fromhex(result["amount_float_bits"]))[0]}})
    if len(mapped) != 99 or len(unmapped) != 2838 or sha256(manifest.encode()) != SUBSET:
        raise ValueError("qualified subset differs from exact 99-case pin")
    return {"format": "ecdev-price-subset-proposal-v1", "scope": "SOURCE_ONLY_PURE_PROBE; NO_CANONICAL_ASSIGNMENT_OR_AUTHORITY",
            "source": {"repository": "llgtrn/ECDEV", "corpus_path": CORPUS, "corpus_sha256": SOURCE_PINS[CORPUS],
                       "implementation_commit": metadata["source_reference"]["commit"],
                       "handoff_commit": metadata["source_reference"]["handoff_commit"],
                       "handoff_path": metadata["source_reference"]["handoff_path"],
                       "handoff_sha256": SOURCE_PINS[HANDOFF], "capture_sha256": SOURCE_PINS[CAPTURE],
                       "source_ids": {"donor": "ec1:7755f6459b7f8f22b262e2bdfe74d7b1", "donor_capability": "ec1:5f41ea7edd845d3559a926b897253666",
                                      "source_capability": "ec1:a2e3b3e884070b64d4edacda08adf6e6", "technology": "ec1:e4aedb10fcd783728d5c4ab221f20132"},
                       "original_819_mapping_sha256": SOURCE_PINS[MAPPING], "original_assignments_changed": 0},
            "target_custody": portable_binding(), "qualified_manifest_sha256": SUBSET,
            "mapped_count": 99, "unmapped_count": 2838, "mapped": mapped, "unmapped": unmapped,
            "canonical_assignment": None, "accepted_owner": None, "authority_grants": [],
            "copying_authorization": "UNKNOWN", "source_adapter_cutover": False,
            "governed_caller_locally_run": False,
            "governed_caller_claim": {"scope": "RETAINED_TARGET_ASSERTION_ONLY; NOT_EXECUTED_HERE", "commit": COMMIT,
                                      "test": "phenora/codara/tests/e2e_price_text.rs::original_pinned_source_corpus_supported_subset_executes_through_governance"}}


def inspect(proposal, expected):
    if canonical(proposal) != canonical(expected):
        raise ValueError("proposal differs from complete independently recomputed expected result")


# These deliberately inert types compile the unchanged parse function. No registry semantics,
# graph selection, Host, authority, lease, evidence or governed caller is exercised by this shim.
WRAPPER = r'''
extern crate self as capability;
extern crate self as genora_primitives;
pub mod action { pub struct SemanticAction; impl TryFrom<&str> for SemanticAction {
 type Error=(); fn try_from(_: &str)->Result<Self,()>{Ok(Self)} } }
pub mod kernel {
 use std::marker::PhantomData; use crate::action::SemanticAction;
 pub trait RequestFacts {} pub enum ExecutionClass {Pure} #[derive(Debug)] pub struct RegistryError;
 pub struct KernelSpec<Q,R>(PhantomData<fn(Q)->R>);
 impl<Q,R> KernelSpec<Q,R> {pub fn new(_:SemanticAction,_:&str,_:u32,_:ExecutionClass,_:fn(&Q)->R)->Self {Self(PhantomData)}}
 pub struct KernelRegistry<Q,R>(PhantomData<fn(Q)->R>);
 impl<Q,R> KernelRegistry<Q,R> {pub fn register(&mut self,_:KernelSpec<Q,R>)->Result<(),RegistryError>{Ok(())}}
}
#[path="price_text.rs"] mod price_text;
use std::io::{self,BufRead};
fn optional(s:&str)->Option<String>{ if s=="-" {return None} assert!(s.len()%2==0);
 let b=(0..s.len()).step_by(2).map(|i|u8::from_str_radix(&s[i..i+2],16).unwrap()).collect();
 Some(String::from_utf8(b).unwrap()) }
fn text(s:Option<&str>)->String {s.map(|v|v.as_bytes().iter().map(|b|format!("{b:02x}")).collect()).unwrap_or("-".into())}
fn main(){for line in io::stdin().lock().lines(){let line=line.unwrap();let f:Vec<_>=line.split('\t').collect();assert_eq!(f.len(),5);
 let i:usize=f[0].parse().unwrap(); let q:Vec<_>=f[1..].iter().map(|v|optional(v)).collect();
 let request=price_text::PriceTextRequest{input:q[0].as_deref(),currency_hint:q[1].as_deref(),decimal_separator:q[2].as_deref(),digit_group_separator:q[3].as_deref()};
 match price_text::parse(&request){Err(e)=>println!("{i}\tERR\t{}",e.code()),Ok(v)=>println!("{i}\tOK\t{}\t{}\t{}\t{}",text(v.amount.as_deref()),text(v.currency.as_deref()),text(v.amount_text.as_deref()),v.amount_float.map(|n|format!("{:016x}",n.to_bits())).unwrap_or("-".into()))}
}}
'''


def parse_results(stdout):
    rows = []
    for line in stdout.decode("ascii").splitlines():
        fields = line.split("\t")
        if len(fields) < 2 or re.fullmatch("0|[1-9][0-9]*", fields[0]) is None:
            raise ValueError("invalid probe index")
        row = {"index": int(fields[0]), "status": fields[1]}
        if len(fields) == 3 and fields[1] == "ERR" and re.fullmatch("[1-9][0-9]?", fields[2]):
            row["code"] = int(fields[2])
        elif len(fields) == 6 and fields[1] == "OK":
            for key, value in zip(FIELDS, fields[2:]):
                row[key] = None if value == "-" else (value if key.endswith("bits") else bytes.fromhex(value).decode())
        else:
            raise ValueError("invalid probe output shape")
        rows.append(row)
    return rows


def encode_requests(corpus):
    def field(value):
        if value is not None and type(value) is not str:
            raise ValueError("source selectors must be optional strings")
        return "-" if value is None else value.encode().hex()
    return "".join("\t".join([str(i), *(field(c[k]) for k in SELECTORS)]) + "\n"
                   for i, c in enumerate(corpus["cases"])).encode()


def separate_output(output, protected):
    for root in protected:
        if output == root or output.is_relative_to(root) or root.is_relative_to(output):
            raise ValueError("output must be physically disjoint from every protected input")
    if output.exists() or output.is_symlink():
        raise ValueError("output must be a new directory; never overwrite prior evidence")


def run(reference, output, source_root, rustc=None, inspect_path=None):
    source, reference, output = (physical_path(p) for p in (source_root, reference, output))
    separate_output(output, [source, reference])
    inspection = None if inspect_path is None else physical_path(inspect_path)
    if inspection is not None:
        separate_output(output, [inspection])
    binding = load_reference(reference)
    corpus, snapshot = load_source(source)
    snapshot.update(binding["snapshot"])
    tool_bytes, tool_mode = read_regular(Path(__file__).resolve())
    snapshot[str(Path(__file__).resolve())] = (sha256(tool_bytes), tool_mode)
    if inspection is not None:
        inspection_bytes, executable = read_regular(inspection)
        snapshot[str(inspection)] = (sha256(inspection_bytes), executable)
    compiler = Path(rustc or shutil.which("rustc") or "")
    if not compiler.is_file() or compiler.resolve().name == "rustup":
        raise ValueError("provide a direct installed rustc executable; no rustup/network fallback")
    compiler = compiler.resolve()
    compiler_bytes, compiler_mode = read_regular(compiler, 256 * 1024 * 1024)
    snapshot[str(compiler)] = (sha256(compiler_bytes), compiler_mode)
    output.mkdir()
    for name in ("tmp", "home"):
        (output / name).mkdir()
    (output / "price_text.rs").write_bytes(binding["source_code"])
    (output / "probe.rs").write_text(WRAPPER)
    env = {"PATH": str(compiler.parent) + ":/usr/bin:/bin", "HOME": str(output / "home"),
           "TMPDIR": str(output / "tmp"), "LANG": "C", "LC_ALL": "C", "TZ": "UTC"}
    commands = []
    def execute(args, data=None):
        commands.append([str(a) for a in args])
        prefix = output / f"command-{len(commands):02}"
        try:
            result = subprocess.run(args, input=data, cwd=output, env=env, capture_output=True, timeout=60, check=False)
        except subprocess.TimeoutExpired as error:
            prefix.with_suffix(".stdout").write_bytes(error.stdout or b"")
            prefix.with_suffix(".stderr").write_bytes(error.stderr or b"")
            raise ValueError(f"command timed out; output preserved at {prefix}") from error
        prefix.with_suffix(".stdout").write_bytes(result.stdout)
        prefix.with_suffix(".stderr").write_bytes(result.stderr)
        if len(result.stdout) + len(result.stderr) > 4 * 1024 * 1024:
            raise ValueError("probe output exceeds budget")
        if result.returncode:
            raise ValueError(f"command failed ({result.returncode}); {prefix}.stderr: " + result.stderr[-4096:].decode(errors="replace"))
        return result
    version = execute([str(compiler), "--version", "--verbose"]).stdout.decode()
    execute([str(compiler), "--edition=2021", "-Awarnings", str(output / "probe.rs"), "-o", str(output / "probe")])
    stdin = encode_requests(corpus)
    stdout = execute([str(output / "probe")], stdin).stdout
    results = parse_results(stdout)
    proposal = build_proposal(corpus, results, binding)
    if inspection is not None:
        inspect(parse_json(inspection_bytes), proposal)
    if load_reference(reference)["snapshot"] != binding["snapshot"]:
        raise ValueError("target reference changed during execution")
    for path, expected in snapshot.items():
        data, executable = read_regular(path, 256 * 1024 * 1024 if path == str(compiler) else 16 * 1024 * 1024)
        if (sha256(data), executable) != expected:
            raise ValueError("qualification input bytes/mode changed during execution")
    def write_json(name, value):
        data = (canonical(value) + "\n").encode()
        (output / name).write_bytes(data)
        return sha256(data)
    results_sha = write_json("results.json", results)
    proposal_sha = write_json("proposal.json", proposal)
    (output / "stdin.tsv").write_bytes(stdin)
    (output / "stdout.tsv").write_bytes(stdout)
    receipt = {"format": "ecdev-price-text-pure-qualification-v1", "status": "PASS", "scope": "PURE_PARSER_WITH_INERT_COMPILATION_STUBS_ONLY",
               "compiler": {"path": str(compiler), "sha256": sha256(compiler_bytes), "version": version}, "commands": commands,
               "wrapper_sha256": sha256(WRAPPER.encode()), "parser_sha256": sha256(binding["source_code"]),
               "binary_sha256": sha256(read_regular(output / "probe")[0]), "stdin_sha256": sha256(stdin), "stdout_sha256": sha256(stdout),
               "results_sha256": results_sha, "proposal_sha256": proposal_sha,
               "input_files": [{"path": p, "sha256": d, "executable": x} for p, (d, x) in sorted(snapshot.items())],
               "stability_scope": "ENUMERATED_FILE_BYTES_AND_EXECUTABLE_MODES_ONLY", "target_custody": portable_binding(),
               "supported": 99, "unmapped": 2838, "governed_caller_locally_run": False,
               "registry_binding": "PENDING", "copying_authorization": "UNKNOWN", "canonical_assignment": None,
               "commit_authentication": "UNSIGNED_GIT_CONTENT_ADDRESS_ONLY; NO_AUTHORIZATION_OR_BUILD_ATTESTATION"}
    receipt["compiler_scope"] = "COMPILER_FILE_AND_OUTPUT_BYTES; SYSROOT_LINKER_DEPENDENCIES_NOT_ATTESTED"
    write_json("qualification.json", receipt)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--rustc", type=Path)
    parser.add_argument("--inspect", type=Path)
    args = parser.parse_args()
    try:
        result = run(args.reference, args.output, args.source_root, args.rustc, args.inspect)
        print(f"PASS: {result['supported']} pure-parser comparisons; {result['unmapped']} UNMAPPED; no admission")
        return 0
    except (ValueError, OSError, KeyError, TypeError, RecursionError, subprocess.SubprocessError) as error:
        print(f"qualification refused: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
