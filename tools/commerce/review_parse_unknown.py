"""Re-review donor source files the census left PARSE_UNKNOWN, from locked blobs.

usage: python tools/commerce/review_parse_unknown.py --checkouts <dir>

<dir>/<donor_id> must be a git clone (a blobless partial clone is enough) that contains the
census commit. Every blob is read by its recorded blob hash and checked against the recorded
sha256 before it is parsed. A file becomes PARSED only when an admissible grammar parses it
with no error node (census.parse_review); every remaining file carries a reason code.
Re-running is idempotent: the first observed "before" state of a file is kept.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/commerce"))
import census  # noqa: E402

CENSUS = ROOT / "research/commerce/donors/census"
AUDIT = ROOT / "research/commerce/parse-unknown-review.json"


def read_jsonl(path):
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--checkouts", required=True, type=pathlib.Path)
    args = parser.parse_args()
    registry_path = ROOT / "research/commerce/donors/registry.json"
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    previous = json.loads(AUDIT.read_text(encoding="utf-8")) if AUDIT.exists() else {"files": []}
    prior = {(f["donor_id"], f["source_path"]): f for f in previous["files"]}
    reviewed = []
    for donor in registry["donors"]:
        directory = CENSUS / donor["donor_id"]
        rows = read_jsonl(directory / "files.jsonl")
        pending = [r for r in rows if r["classification"] == "FIRST_PARTY_SOURCE" and r.get("parse_status") != "PARSED"]
        pending += [r for r in rows if (donor["donor_id"], r["path"]) in prior and r not in pending]
        if not pending:
            continue
        summary = json.loads((directory / "summary.json").read_text(encoding="utf-8"))
        checkout = args.checkouts / donor["donor_id"]
        symbols = read_jsonl(directory / "symbols.jsonl")
        graph = json.loads((directory / "dependency-graph.json").read_text(encoding="utf-8"))
        for row in pending:
            blob = subprocess.check_output(["git", "cat-file", "blob", row["blob_hash"]], cwd=checkout)
            assert hashlib.sha256(blob).hexdigest() == row["sha256"], (donor["donor_id"], row["path"])
            committed = subprocess.check_output(["git", "rev-parse", f"{summary['commit_sha']}:{row['path']}"], cwd=checkout, text=True).strip()
            assert committed == row["blob_hash"], (donor["donor_id"], row["path"])
            saved = prior.get((donor["donor_id"], row["path"]))
            before = saved["before"] if saved else {"parse_status": row.get("parse_status"), "language": row.get("language")}
            kind = census.classify(row["path"], blob)
            if kind != row["classification"] or kind != "FIRST_PARTY_SOURCE":
                # Generated output (e.g. Cython's C++) is a build artifact, not first-party source.
                before.setdefault("classification", row["classification"])
                row["classification"] = kind
                row["classification_reason"] = "Generated-file banner in the locked blob"
                for key in ("parse_status", "parse_reason", "parse_reason_code", "parse_grammar", "parse_method"):
                    row.pop(key, None)
                reviewed.append({
                    "donor_id": donor["donor_id"], "commit_sha": summary["commit_sha"], "source_path": row["path"],
                    "blob_hash": row["blob_hash"], "sha256": row["sha256"], "before": before,
                    "after": {"classification": kind, "parse_status": None, "language": row.get("language"), "grammar": None, "method": "RECLASSIFIED_GENERATED"},
                    "reason_code": None, "first_error": None,
                })
                continue
            result = census.parse_review(row["path"], blob)
            row["parse_status"] = result["status"]
            row["language"] = result["language"]
            if result["status"] == "PARSED":
                # A clean parse supersedes symbols and imports from the earlier error-tolerant parse;
                # an unresolved file keeps them untouched.
                symbols = [s for s in symbols if s["source_path"] != row["path"]] + result["symbols"]
                graph["imports"] = [i for i in graph.get("imports", []) if i.get("source_path") != row["path"]] + result["imports"]
            for key in ("parse_reason", "parse_reason_code", "parse_grammar", "parse_method"):
                row.pop(key, None)
            if result["status"] == "PARSED":
                row["parse_grammar"] = result["grammar"]
                row["parse_method"] = result["method"]
            else:
                row["parse_reason_code"] = result["reason_code"]
                row["parse_reason"] = "No admissible grammar parses this file without error nodes; retained as parse_unknown"
            reviewed.append({
                "donor_id": donor["donor_id"], "commit_sha": summary["commit_sha"], "source_path": row["path"],
                "blob_hash": row["blob_hash"], "sha256": row["sha256"], "before": before,
                "after": {"parse_status": result["status"], "language": result["language"], "grammar": result["grammar"], "method": result["method"]},
                "reason_code": result["reason_code"], "first_error": result["error"],
            })
        source = [r for r in rows if r["classification"] == "FIRST_PARTY_SOURCE"]
        unknown = [r for r in source if r.get("parse_status") != "PARSED"]
        counts = collections.Counter(r["classification"] for r in rows)
        summary.update(classification_counts=dict(counts), first_party_source_files=len(source),
                       source_parsed=len(source) - len(unknown), parse_unknown=len(unknown), symbols=len(symbols))
        donor["source_census_status"] = "STRUCTURAL_PARSED" if not unknown else "PARTIAL"
        donor["primary_languages"] = dict(collections.Counter(r["language"] for r in rows if r.get("language")))
        census.jsonl(directory / "files.jsonl", rows)
        census.jsonl(directory / "symbols.jsonl", symbols)
        census.dump(directory / "dependency-graph.json", graph)
        census.dump(directory / "summary.json", summary)
        census.dump(directory / "identity.json", donor)
        census.dump(directory / "languages.json", donor["primary_languages"])
        risks = json.loads((directory / "risks.json").read_text(encoding="utf-8"))
        risks["parse_unknown"] = [r["path"] for r in unknown]
        risks["parse_unknown_reasons"] = {r["path"]: r["parse_reason_code"] for r in unknown}
        census.dump(directory / "risks.json", risks)
    census.dump(registry_path, registry)
    resolved = [f for f in reviewed if f["after"]["parse_status"] == "PARSED" or f["after"]["method"] == "RECLASSIFIED_GENERATED"]
    census.dump(AUDIT, {
        "scope": "Donor source files the census left PARSE_UNKNOWN, re-read from locked blobs (blob hash and sha256 verified)",
        "parser": "tree-sitter 0.25.2 / tree-sitter-language-pack 0.10.0 (tools/commerce/requirements.txt)",
        "admissibility": [
            "PARSED requires a parse tree with no ERROR or MISSING node",
            "fallback grammars only where the grammar accepts a superset of the file language (SCSS for CSS, TSX for JSX) or a language a .h header can hold (C++, Objective-C)",
            "template grammars are never fallbacks: they accept nearly any text",
            "notebooks: Python code cells only, IPython magic and shell-escape lines blanked in place",
            "reason codes label a failure; they never turn it into a parse",
        ],
        "semantic_completion": False,
        "reviewed": len(reviewed),
        "resolved": len(resolved),
        "remaining": len(reviewed) - len(resolved),
        "resolved_by_method": dict(collections.Counter(f["after"]["method"] for f in resolved)),
        "remaining_by_reason": dict(collections.Counter(f["reason_code"] for f in reviewed if f not in resolved)),
        "files": reviewed,
    })
    print(f"reviewed {len(reviewed)}; resolved {len(resolved)}; remaining {len(reviewed) - len(resolved)}")


if __name__ == "__main__":
    main()
