"""Pinned source intake for memory contract research; no runtime integration."""
from __future__ import annotations
import argparse, json, hashlib, sys
from pathlib import Path
import census
from intake_social import verify

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("identity", choices=["getzep/zep","getzep/graphiti"])
    parser.add_argument("--defer-registry",action="store_true")
    args=parser.parse_args()
    registry_path=census.YN/"donors/registry.json"
    original=registry_path.read_bytes()
    verification=verify(args.identity)
    if verification["status"] != "VERIFIED_REMOTE":
        raise RuntimeError("Exact remote identity unavailable: "+json.dumps(verification))
    record=census.acquire(args.identity)
    if record["clone_status"] != "FULL_CLONE" or record["commit_sha"] != verification["remote_head_at_verification"]:
        raise RuntimeError("Full clone or pinned remote HEAD mismatch; inspect retained intake records")
    checkout=census.YN/"donors/checkouts"/record["donor_id"]
    submodules=census.git("submodule","status","--recursive",cwd=checkout)
    if any(line[:1] in {"-","+","U"} for line in submodules.splitlines()):
        raise RuntimeError("Submodule checkout incomplete or mismatched")
    record["memory_identity_verification"]=verification
    record["submodule_status"]=submodules
    record["license_absorption_gate"]="PENDING_LOCKED_SOURCE_REVIEW_NO_PRODUCTION_COPY"
    census.dump(census.YN/"donors/census"/record["donor_id"]/"identity.json",record)
    census.dump(census.YN/(args.identity.split("/")[1]+"-intake.json"),{"verification":verification,"record":record,"submodule_status":submodules,"runtime_dependencies_added":0})
    if args.defer_registry:
        print("Pinned census retained; registry deferred",record["donor_id"],record["commit_sha"],record["file_count"],flush=True)
        return
    if hashlib.sha256(registry_path.read_bytes()).digest()!=hashlib.sha256(original).digest():
        raise RuntimeError("Concurrent registry edit; intake retained without registry overwrite")
    registry=json.loads(original)
    existing={r["donor_id"] for r in registry["donors"]}
    if record["donor_id"] not in existing: registry["donors"].append(record)
    registry["generated_at"]=census.now()
    census.dump(registry_path,registry)
    census.dump(census.YN/(args.identity.split("/")[1]+"-intake.json"),{"verification":verification,"record":record,"submodule_status":submodules,"runtime_dependencies_added":0})
    print("Pinned",record["donor_id"],record["commit_sha"],record["file_count"],flush=True)

if __name__=="__main__": main()
