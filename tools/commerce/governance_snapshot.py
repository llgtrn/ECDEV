"""Record ECDEV governance state (lifecycle, conformance, metrics) as committed evidence.

Runs the ECDEV-native governance binary against the repository and writes
research/commerce/governance/{lifecycle,conformance}.json, then refreshes the per-donor
assessments in research/commerce/donor-lifecycle-review.json. Nothing here reads or names
any external governance system; the binary is built from .ecdev/ alone.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "research/commerce/governance"


def gov(*args):
    run = subprocess.run(
        ["cargo", "run", "--quiet", "--release", "--manifest-path", str(ROOT / ".ecdev/Cargo.toml"), "--", *args, "--root", str(ROOT), "--json"],
        capture_output=True, text=True,
    )
    if not run.stdout.strip():
        sys.exit(f"ecdev-gov {' '.join(args)} produced no output: {run.stderr}")
    return run.returncode, json.loads(run.stdout)


def main():
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip())
    _, extinction = gov("extinction")
    code, conformance = gov("conformance")
    base = {"authority": ".ecdev", "assessed_parent_commit": head, "working_tree_dirty_at_assessment": dirty,
            "schema": extinction["schema"]}
    donors = extinction["donors"]
    states = {}
    for d in donors:
        states[d["effective"]] = states.get(d["effective"], 0) + 1
    lifecycle = {**base, "source": "ecdev-gov extinction --json", "donor_count": len(donors),
                 "effective_states": dict(sorted(states.items())),
                 "extinct": sorted(d["donor"] for d in donors if d["effective"] == "EXTINCT"),
                 "donors": donors}
    checks = conformance["checks"]
    report = {**base, "suite": "ECDEV REPOSITORY CONFORMANCE", "source": "ecdev-gov conformance --json",
              "exit_code": code, "passed": sum(c["pass"] for c in checks), "total": len(checks),
              "failing": [c for c in checks if not c["pass"]], "checks": checks,
              "v1_gate": conformance["v1_gate"], "metrics": conformance["metrics"]}
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "lifecycle.json").write_text(json.dumps(lifecycle, indent=2) + "\n", encoding="utf-8")
    (OUT / "conformance.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    review_path = ROOT / "research/commerce/donor-lifecycle-review.json"
    review = json.loads(review_path.read_text(encoding="utf-8"))
    by_id = {d["donor"]: d for d in donors}
    review["canonical_authority"] = ".ecdev/src/donors/mod.rs"
    for row in review["donors"]:
        d = by_id.get(row["donor_id"])
        if d:
            row["canonical_assessment"] = {"donor_id": d["donor"], "claimed": d["claimed"], "effective": d["effective"],
                                           "stopped_by": d["stopped_by"], "extinct": d["effective"] == "EXTINCT"}
            row["extinct"] = d["effective"] == "EXTINCT"
    review["assessment_refresh"] = f"ecdev-gov extinction at parent {head}"
    review_path.write_text(json.dumps(review, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"conformance": f"{report['passed']}/{report['total']}", "effective_states": lifecycle["effective_states"],
                      "extinct": lifecycle["extinct"]}))


if __name__ == "__main__":
    main()
