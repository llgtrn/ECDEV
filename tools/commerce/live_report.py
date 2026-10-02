"""Summarize a persisted live capture without relabeling fixtures or unknowns."""
import hashlib
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def main():
    result_path = pathlib.Path(sys.argv[1])
    result = json.loads(result_path.read_text(encoding="utf-8-sig"))
    assert result["mode"] == "LIVE"
    assert result["cost_minor"] == 0
    assert all(o["mode"] == "LIVE" for o in result["observations"])
    assert all(c["provider"] == "native-web" for c in result["provider_calls"])
    snapshots = result["snapshots"]
    products = [c["product"] for c in result["candidates"]]
    captures = []
    for snapshot in snapshots:
        digest = snapshot["content_hash"]
        raw = ROOT / ".ynventa/materialized/raw" / (digest + ".html")
        assert hashlib.sha256(raw.read_bytes()).hexdigest() == digest
        captures.append({"source": snapshot["source"], "sha256": digest,
                         "product_records": len(snapshot["products"])})
    fields = ["title", "brand", "sku", "price_minor", "currency", "availability",
              "seller", "weight_g", "sales", "search_volume", "supplier_moq",
              "shipping_cost", "supplier_capacity"]
    observed = {field: sum(p.get(field) is not None for p in products) for field in fields}
    denominator = len(products) * len(fields)
    record = {
        "status": "LIVE_ACQUISITION_PROOF_ONLY_NOT_FULL_WAVE_COMPLETION",
        "run_id": result["run_id"], "crawl_run_id": result["crawl_run_id"],
        "mode": result["mode"], "country": "JP", "market": result["market"],
        "result_status": result["status"], "fixture_injection": False,
        "urls_discovered": sum(result["frontier"]["states"].values()),
        "pages_captured": len(snapshots),
        "pages_with_product_metadata": sum(bool(s["products"]) for s in snapshots),
        "recognition_method": "JSON_LD_PRODUCT_METADATA_NOT_FULL_CLASSIFIER",
        "candidate_observation_records": len(products),
        "unique_resolved_products": None,
        "funnel": result["funnel"], "frontier": result["frontier"],
        "known_network_calls": result["known_network_calls"],
        "total_network_calls": result["network_calls"],
        "paid_provider_calls": 0, "paid_cost_minor": 0,
        "blocked_routes": result["errors"],
        "candidate_field_observability": {
            "unit": "CANDIDATE_OBSERVATION_RECORD_NOT_UNIQUE_PRODUCT",
            "fields": fields, "non_null_by_field": observed,
            "observed_assertions": sum(observed.values()),
            "unknown_assertions": denominator - sum(observed.values()),
            "denominator": denominator,
            "completeness_bps": sum(observed.values()) * 10000 // denominator if denominator else 0,
            "scope": "Declared fields only; source assertions are not independent validation",
        },
        "fixture_stage_coverage": "SEPARATE_EXECUTABILITY_METRIC_NOT_LIVE_COMPLETENESS",
        "end_to_end_shortlist_proof": "NOT_PASSED",
        "remaining": ["Entity resolution", "Independent competition and demand proxies",
                      "Live supplier evidence", "Economics uncertainty", "Shortlist"],
        "captures": captures,
    }
    output = ROOT / "research/commerce/live-jp-acquisition-proof.json"
    output.write_text(json.dumps(record, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in record.items() if k != "captures"}, indent=2))


if __name__ == "__main__":
    main()
