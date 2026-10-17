#!/usr/bin/env python3
"""Reproduce the retained 106-case index/name manifest without granting authority.

The caller must independently supply the retained sidecar digest. This bounded
producer additionally fixes the original corpus and sidecar bytes; a caller pin
cannot rebind them. No source outcomes are normalized, rewritten or executed.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import sys


CORPUS_PATH = "adapter/web/tests/fixtures/price-contract.json"
SIDECAR_PATH = "research/commerce/governance/price-bounded-mapping-db92bf27-v1.json"
MANIFEST_PATH = "research/commerce/governance/price-supported-106-manifest-db92bf27-v1.tsv"
CORPUS_SHA256 = "3729e24b33e660f6155ccca4ef5e8b199de4b2cab1e7b517cf403f930157d3ce"
SIDECAR_SHA256 = "300591b5db1f9b156ecf5282525e5d056c79986140cffff20fcd784b2e825792"
MANIFEST_SHA256 = "451f0bb1a2f3acd3879ddc2945c14e74447ec6ac70dfd5ebf374d086c8f353dc"
OLD99_SHA256 = "d5794485dd05bcb51e79fcc5af2a0ea6a1a915800a829ee0211cd3df985df29f"
NEW7_SHA256 = "5beaf6ab248dde7a3c42e6ef1b177c64e8c3af481561e5dd2d76aa2d893d342e"
NEW_INDICES = (117, 212, 409, 855, 1188, 1193, 1849)
MAX_INPUT = 16 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def _pinned_json(raw, expected, label):
    if type(raw) is not bytes or len(raw) > MAX_INPUT or digest(raw) != expected:
        raise ValueError(f"{label} bytes differ from fixed retained pin")

    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result

    def invalid(value):
        raise ValueError("nonfinite JSON token: " + value)

    # Valid source exponent spellings may decode to nonfinite advisory floats.
    # They are never serialized or used for index/name manifest selection.
    return json.loads(raw.decode("utf-8"), object_pairs_hook=unique, parse_constant=invalid)


def build_manifest(corpus_bytes, sidecar_bytes, expected_sidecar_sha256):
    """Return exact UTF-8 ``index<TAB>original name<LF>`` bytes for the fixed cohort."""
    if type(expected_sidecar_sha256) is not str or expected_sidecar_sha256 != SIDECAR_SHA256:
        raise ValueError("independent caller pin must identify the retained bounded sidecar")
    sidecar = _pinned_json(sidecar_bytes, expected_sidecar_sha256, "sidecar")
    corpus = _pinned_json(corpus_bytes, CORPUS_SHA256, "corpus")
    cases = corpus["cases"]
    coverage = sidecar["coverage"]
    indices = coverage["qualified_case_indices"]
    if (type(cases) is not list or len(cases) != 2937
            or type(indices) is not list or len(indices) != 106
            or any(type(i) is not int or not 0 <= i < len(cases) for i in indices)
            or indices != sorted(set(indices))
            or (coverage["total"], coverage["supported"], coverage["unmapped"]) != (2937, 106, 2831)):
        raise ValueError("retained ordered 106-case cohort required")

    def manifest(selected):
        rows = []
        for index in selected:
            name = cases[index]["name"]
            if type(name) is not str or any(c in name for c in "\t\r\n\0"):
                raise ValueError("original case name must fit one manifest row")
            rows.append(f"{index}\t{name}\n")
        return "".join(rows).encode("utf-8")

    additions = [row["index"] for row in sidecar["new_candidates"]]
    if (any(type(i) is not int for i in additions) or tuple(additions) != NEW_INDICES
            or not set(NEW_INDICES).issubset(indices)
            or digest(manifest(NEW_INDICES)) != NEW7_SHA256
            or digest(manifest([i for i in indices if i not in NEW_INDICES])) != OLD99_SHA256):
        raise ValueError("original 99 plus the exact seven retained candidates required")
    result = manifest(indices)
    if digest(result) != MANIFEST_SHA256 or coverage["all106_manifest_sha256"] != MANIFEST_SHA256:
        raise ValueError("exact retained manifest digest required")
    return result


def inspect_manifest(corpus_bytes, sidecar_bytes, expected_sidecar_sha256, manifest_bytes):
    """Check exact bytes, including row order and final LF; return metadata only."""
    expected = build_manifest(corpus_bytes, sidecar_bytes, expected_sidecar_sha256)
    if type(manifest_bytes) is not bytes or manifest_bytes != expected:
        raise ValueError("manifest differs from exact retained 106-case bytes")
    return {"format": "ecdev-price-supported-manifest-inspection-v1",
            "scope": "INDEX_NAME_METADATA_ONLY; NO_CANONICAL_ASSIGNMENT_OR_ADMISSION",
            "corpus_sha256": CORPUS_SHA256, "sidecar_sha256": SIDECAR_SHA256,
            "manifest_sha256": MANIFEST_SHA256, "manifest_bytes": len(expected),
            "total": 2937, "supported": 106, "unmapped": 2831,
            "old99_manifest_sha256": OLD99_SHA256, "new_candidate_indices": list(NEW_INDICES)}


def read_regular(path):
    """Read bounded regular input without following links or opening a writer."""
    path = Path(path).absolute()
    if ".." in path.parts or any(p.is_symlink() for p in (*path.parents, path)):
        raise ValueError("input path contains a link or parent traversal")
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as stream:
        info = os.fstat(stream.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_INPUT:
            raise ValueError("input must be a bounded regular file")
        raw = stream.read(MAX_INPUT + 1)
    if len(raw) > MAX_INPUT:
        raise ValueError("input exceeds byte budget")
    return raw


def main():
    root = Path(__file__).absolute().parents[2]
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--corpus", type=Path, default=root / CORPUS_PATH)
    parser.add_argument("--sidecar", type=Path, default=root / SIDECAR_PATH)
    parser.add_argument("--expected-sidecar-sha256", required=True)
    parser.add_argument("--inspect-manifest", type=Path)
    args = parser.parse_args()
    try:
        corpus, sidecar = read_regular(args.corpus), read_regular(args.sidecar)
        if args.inspect_manifest is None:
            sys.stdout.buffer.write(build_manifest(corpus, sidecar, args.expected_sidecar_sha256))
        else:
            result = inspect_manifest(corpus, sidecar, args.expected_sidecar_sha256,
                                      read_regular(args.inspect_manifest))
            print(json.dumps(result, sort_keys=True, allow_nan=False))
    except (OSError, ValueError) as error:
        parser.exit(2, f"price manifest: {error}\n")


if __name__ == "__main__":
    main()
