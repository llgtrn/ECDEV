#!/usr/bin/env python3
"""Read-only comparison of sealed portable data; no compiler, caller or admission runs."""
import argparse
import hashlib
import math
import os
from pathlib import Path
import re
import stat
import struct
import sys

CORE_SHA = "9c1c062814b92d1049628b63c32f07fec6bb46e749a7c6c884a284838f0e3bba"
HANDOFF_PATH = "research/commerce/governance/price-target-published-handoff-v1.json"
MAP_PATH = "research/commerce/governance/price-target-published-case-map-v1.jsonl"
RESULTS_PATH = "research/commerce/governance/price-text-pure-probe-v1.json"
PROPOSAL_PATH = "research/commerce/governance/price-text-subset-proposal-v1.json"
PINS = {
    HANDOFF_PATH: "53737eac4519add6701f20a63385db5a82a61b175253779c93cb060e2c542f02",
    MAP_PATH: "a320f5e062cabe3c8fa36fdc214a6298d9b84769f1a37da3487ac11ec5116a93",
    RESULTS_PATH: "edfd20a74dec11d8ff370d4ab5d859b54de9def6fc75c1caf43b6719cbe6754b",
    PROPOSAL_PATH: "2e05d72ca01d42bc537bbfba8a9c03c398008e495a5abf5af6e032110923e9cf",
}
HANDOFF_CANONICAL_SHA = "73f4d11d67cd3fe9f5ffcb3c60bdf14a2c98675decd77a224223cf02539705d5"


def helpers():
    path = Path(__file__).resolve().with_name("price_subset.py")
    def sealed_bytes():
        with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size != 25977:
                raise ValueError("original pure qualification helper is not the bounded regular file")
            data = stream.read(25978)
        if len(data) != 25977 or hashlib.sha256(data).hexdigest() != CORE_SHA:
            raise ValueError("original pure qualification helper changed")
        return data, stat.S_IMODE(info.st_mode)
    before = sealed_bytes()
    # Execute only the sealed bytes, with no second loader read or bytecode cache.
    core = {"__file__": str(path), "__name__": "sealed_price_subset"}
    exec(compile(before[0], str(path), "exec"), core)
    if sealed_bytes() != before:
        raise ValueError("pure helper changed while loading")
    return core


def encode_string(value):
    if type(value) is not str:
        raise ValueError("expected exact UTF-8 string")
    raw = value.encode()
    if len(raw) > 0xffffffff:
        raise ValueError("string exceeds u32 wire length")
    return struct.pack("<I", len(raw)) + raw


def optional_string(value):
    return b"\0" if value is None else b"\1" + encode_string(value)


def request_digest(case, domain):
    wire = encode_string(domain) + b"".join(optional_string(case[key]) for key in
        ("input", "currency_hint", "decimal_separator", "digit_group_separator"))
    return hashlib.sha256(wire).hexdigest()


def projection_digest(fields, domain):
    wire = encode_string(domain) + b"".join(optional_string(fields[key]) for key in ("amount", "currency", "amount_text"))
    if "amount_float" in fields and fields["amount_float"] is not None and type(fields["amount_float"]) not in (float, int):
        raise ValueError("invalid advisory float type")
    if "amount_float_bits" in fields:
        bits = fields["amount_float_bits"]
        if bits is not None and (type(bits) is not str or re.fullmatch("[0-9a-f]{16}", bits) is None):
            raise ValueError("invalid IEEE binary64 bits")
        raw = None if bits is None else int(bits, 16).to_bytes(8, "little")
        if "amount_float" in fields:
            value = fields["amount_float"]
            actual = None if value is None else struct.pack("<d", value)
            if actual != raw:
                raise ValueError("advisory float and bits disagree")
    else:
        value = fields["amount_float"]
        if value is not None and type(value) not in (float, int):
            raise ValueError("invalid advisory float type")
        raw = None if value is None else struct.pack("<d", value)
    if raw is not None and not math.isfinite(struct.unpack("<d", raw)[0]):
        raise ValueError("nonfinite advisory float")
    return hashlib.sha256(wire + (b"\0" if raw is None else b"\1" + raw)).hexdigest()


def validate_handoff(handoff):
    core = helpers()
    if core["sha256"](core["canonical"](handoff).encode()) != HANDOFF_CANONICAL_SHA:
        raise ValueError("published handoff identity, rights or claims changed")


def validate_rows(corpus, results, rows, handoff):
    validate_handoff(handoff)
    core = helpers()
    if core["sha256"]((core["canonical"](results) + "\n").encode()) != PINS[RESULTS_PATH]:
        raise ValueError("actual pure-probe transcript changed")
    core["build_proposal"](corpus, results, core["portable_binding"]())
    if type(rows) is not list or len(rows) != 2937:
        raise ValueError("all 2937 ordered published case rows required")
    mapping = handoff["source_case_mapping"]
    def domain(field):
        matched = re.search(r"str\(([^)]+)\)", mapping[field])
        if matched is None:
            raise ValueError("missing sealed digest domain")
        return matched.group(1)
    request_domain, projection_domain = domain("request_digest"), domain("projection_digest")
    supported, unmapped, codes = [], [], {}
    for index, (row, case, result) in enumerate(zip(rows, corpus["cases"], results)):
        if type(row) is not dict or type(row.get("index")) is not int or row["index"] != index:
            raise ValueError("duplicate, reordered or wrong-typed case index")
        if (row.get("request_sha256") != request_digest(case, request_domain)
                or row.get("source_projection_sha256") != projection_digest(case["expected"], projection_domain)):
            raise ValueError("published request or source projection digest mismatch")
        keys = {"index", "request_sha256", "source_projection_sha256", "native"}
        if result["status"] == "OK":
            if (set(row) != keys | {"result_sha256"} or row["native"] != "SUPPORTED"
                    or row["result_sha256"] != projection_digest(result, projection_domain)
                    or row["result_sha256"] != row["source_projection_sha256"] or case["error"] is not None):
                raise ValueError("supported projection, status or source error mismatch")
            supported.append(index)
        else:
            if (set(row) != keys | {"unavailable"} or row["native"] != "UNMAPPED"
                    or type(row["unavailable"]) is not int or row["unavailable"] != result["code"]):
                raise ValueError("unmapped status or typed unavailability code mismatch")
            unmapped.append(index)
            code = str(row["unavailable"])
            codes[code] = codes.get(code, 0) + 1
    manifest = "".join(f"{i}\t{corpus['cases'][i]['name']}\n" for i in supported).encode()
    if supported != mapping["supported_indices"] or len(supported) != 99 or len(unmapped) != 2838 or core["sha256"](manifest) != core["SUBSET"]:
        raise ValueError("published supported subset drift")
    return {"total": 2937, "supported": 99, "unmapped": 2838, "qualified_case_indices": supported,
            "unmapped_case_indices": unmapped, "unavailability_codes": codes, "qualified_manifest_sha256": core["SUBSET"]}


def validate(corpus, results, proposal, handoff, rows):
    core = helpers()
    expected = core["build_proposal"](corpus, results, core["portable_binding"]())
    core["inspect"](proposal, expected)
    return validate_rows(corpus, results, rows, handoff)


def reconcile(root=None):
    core = helpers()
    root = Path(__file__).resolve().parents[2] if root is None else root
    root = core["physical_path"](root)
    corpus, snapshot = core["load_source"](root)
    raw = {}
    for relative, digest in PINS.items():
        data, executable = core["read_regular"](root / relative)
        if core["sha256"](data) != digest:
            raise ValueError(f"retained published comparison input changed: {relative}")
        raw[relative] = data
        snapshot[str(root / relative)] = (digest, executable)
    handoff, results, proposal = (core["parse_json"](raw[path]) for path in (HANDOFF_PATH, RESULTS_PATH, PROPOSAL_PATH))
    checked = validate(corpus, results, proposal, handoff, [core["parse_json"](line) for line in raw[MAP_PATH].splitlines()])
    for path, expected in snapshot.items():
        data, executable = core["read_regular"](path)
        if (core["sha256"](data), executable) != expected:
            raise ValueError("comparison input changed during execution")
    return {"format": "source-portable-handoff-reconciliation-v1", "scope": "SEALED_DATA_COMPARISON_ONLY; NO_COMPILATION_OR_CALLER_EXECUTION",
            **checked, "input_sha256": {**core["SOURCE_PINS"], **PINS}, "pure_helper_sha256": CORE_SHA,
            "source_ids": proposal["source"]["source_ids"], "published_candidate": handoff["target_binding"],
            "original_819_assignments_changed": 0, "canonical_assignment": None, "accepted_owner": None,
            "copying_authorization": "UNKNOWN", "registry_binding": "PENDING", "source_adapter_cutover": False,
            "target_activation": handoff["target_binding"]["activation"], "governed_caller_locally_run": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[2])
    try:
        print(helpers()["canonical"](reconcile(parser.parse_args().source_root)))
    except (ValueError, OSError, KeyError, TypeError, OverflowError, RecursionError) as error:
        print(f"comparison refused: {error}", file=sys.stderr)
        sys.exit(2)
