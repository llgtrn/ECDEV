"""Reparse locked unknown-tail blobs without resetting reviewed donor semantics."""
import collections
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / ".venv/research"))
import census


def main():
    registry_path = ROOT / "research/commerce/donors/registry.json"
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    audit_path = ROOT / "research/commerce/census-tail-review.json"
    previous = json.loads(audit_path.read_text(encoding="utf-8")) if audit_path.exists() else {"changes": []}
    prior = {(c["donor_id"], c["source_path"]): c for c in previous["changes"]}
    changes = []
    for donor in registry["donors"]:
        directory = ROOT / "research/commerce/donors/census" / donor["donor_id"]
        checkout = ROOT / "research/commerce/donors/checkouts" / donor["donor_id"]
        assert census.git("rev-parse", "HEAD", cwd=checkout) == donor["commit_sha"]
        rows = [json.loads(line) for line in (directory / "files.jsonl").read_text(encoding="utf-8").splitlines()]
        symbols = [json.loads(line) for line in (directory / "symbols.jsonl").read_text(encoding="utf-8").splitlines()]
        dependency_graph = json.loads((directory / "dependency-graph.json").read_text(encoding="utf-8"))
        changed = False
        for row in rows:
            saved = prior.get((donor["donor_id"], row["path"]))
            if saved is None and row["classification"] != "UNKNOWN" and not (row["classification"] == "FIRST_PARTY_SOURCE" and row.get("parse_status") != "PARSED"):
                continue
            before = saved["before"] if saved else {"classification": row["classification"], "parse_status": row.get("parse_status")}
            blob = subprocess.check_output(["git", "cat-file", "blob", row["blob_hash"]], cwd=checkout)
            assert hashlib.sha256(blob).hexdigest() == row["sha256"]
            row["classification"] = census.classify(row["path"], blob)
            row["classification_reason"] = "Locked blob and explicit format/path/shebang classification; not semantic completion"
            if row["classification"] == "FIRST_PARTY_SOURCE":
                status, fresh_symbols, imports = census.parse_source(row["path"], blob)
                row["parse_status"] = status
                row["language"] = census.source_language(row["path"], blob)
                if status == "PARSED":
                    symbols = [s for s in symbols if s["source_path"] != row["path"]] + fresh_symbols
                    row.pop("parse_reason", None)
                    dependency_graph["imports"] = [i for i in dependency_graph.get("imports", []) if i.get("source_path") != row["path"]] + imports
                else:
                    row["parse_reason"] = next((i["parse_error"] for i in imports if "parse_error" in i), "UNSUPPORTED_GRAMMAR_OR_SYNTAX_ERRORS; retained as parse_unknown")
            after = {"classification": row["classification"], "parse_status": row.get("parse_status")}
            changes.append({"donor_id": donor["donor_id"], "commit_sha": donor["commit_sha"], "source_path": row["path"], "blob_hash": row["blob_hash"], "before": before, "after": after, "remaining_reason": row.get("parse_reason")})
            changed = True
        if changed:
            census.jsonl(directory / "files.jsonl", rows)
            census.jsonl(directory / "symbols.jsonl", symbols)
            census.dump(directory / "dependency-graph.json", dependency_graph)
            summary = json.loads((directory / "summary.json").read_text(encoding="utf-8"))
            counts = collections.Counter(r["classification"] for r in rows)
            source = [r for r in rows if r["classification"] == "FIRST_PARTY_SOURCE"]
            summary.update(classification_counts=dict(counts), classified_files=len(rows)-counts["UNKNOWN"], unknown_files=counts["UNKNOWN"], first_party_source_files=len(source), source_parsed=sum(r.get("parse_status")=="PARSED" for r in source), parse_unknown=sum(r.get("parse_status")!="PARSED" for r in source), symbols=len(symbols))
            if summary["status"] != "CENSUS_COMPLETE":
                summary["status"] = "CENSUS_PARTIAL"
            donor["tree_status"] = "CLASSIFIED" if not summary["unknown_files"] else "PARTIAL"
            donor["source_census_status"] = "STRUCTURAL_PARSED" if not summary["parse_unknown"] else "PARTIAL"
            donor["primary_languages"] = dict(collections.Counter(r["language"] for r in rows if r.get("language")))
            census.dump(directory / "summary.json", summary)
            census.dump(directory / "identity.json", donor)
            census.dump(directory / "languages.json", dict(collections.Counter(r["language"] for r in rows if r.get("language"))))
            risks = json.loads((directory / "risks.json").read_text(encoding="utf-8"))
            risks["parse_unknown"] = [r["path"] for r in source if r.get("parse_status") != "PARSED"]
            census.dump(directory / "risks.json", risks)
            build = json.loads((directory / "build.json").read_text(encoding="utf-8"))
            build["files"] = [r for r in rows if r["classification"] == "BUILD"]
            census.dump(directory / "build.json", build)
    census.dump(registry_path, registry)
    census.dump(ROOT / "research/commerce/census-tail-review.json", {"scope": "Locked unknown tail only; original 77 unclassified files and 193 parse_unknown files", "semantic_completion": False, "changes": changes})
    print("Reviewed locked tail records:", len(changes))


if __name__ == "__main__":
    main()
