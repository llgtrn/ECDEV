"""Verify frozen Amazon examples against their unmodified locked OpenAPI schemas."""
import copy
import hashlib
import json
from pathlib import Path
import warnings

with warnings.catch_warnings():
    warnings.simplefilter("ignore", DeprecationWarning)
    from jsonschema import Draft4Validator, RefResolver

ROOT = Path(__file__).resolve().parents[2]


def main():
    directory = ROOT / "adapter/marketplace/tests/fixtures"
    fixture = json.loads((directory / "official-responses.json").read_text(encoding="utf-8"))
    validators = {}
    values = {}
    for case in fixture["cases"]:
        key = case["operation"]
        path = directory / f"{key}-model.json"
        assert hashlib.sha256(path.read_bytes()).hexdigest() == case["source_sha256"]
        model = json.loads(path.read_bytes())
        source = model
        for part in case["source_pointer"].strip("/").split("/"):
            part = part.replace("~1", "/").replace("~0", "~")
            source = source[int(part)] if isinstance(source, list) else source[part]
        value = json.loads(case["response"]["raw_body"])
        assert source == value, "FROZEN_EXAMPLE_DIFFERS_FROM_UNMODIFIED_MODEL"
        validator = Draft4Validator(case["response_schema"], resolver=RefResolver.from_schema(model))
        validator.validate(value)
        validators[key] = validator
        values[key] = value
    mutants = []
    catalog = copy.deepcopy(values["catalog"])
    del catalog["asin"]
    mutants.append(("catalog", catalog))
    for field, replacement in [("SubCondition", None), ("IsFulfilledByAmazon", "false")]:
        offers = copy.deepcopy(values["offers"])
        if replacement is None:
            del offers["payload"]["Offers"][0][field]
        else:
            offers["payload"]["Offers"][0][field] = replacement
        mutants.append(("offers", offers))
    fees = copy.deepcopy(values["fees"])
    fees["payload"]["FeesEstimateResult"]["FeesEstimate"]["TotalFeesEstimate"]["Amount"] = "3.0"
    mutants.append(("fees", fees))
    for key, value in mutants:
        assert list(validators[key].iter_errors(value)), "SCHEMA_NEGATIVE_CASE_NOT_REJECTED"
    report = {
        "status": "PASS", "donor": fixture["donor"], "commit_sha": fixture["commit"],
        "positive_examples": len(values), "negative_schema_cases": len(mutants),
        "method": "UNMODIFIED_LOCKED_MODELS_AND_EXAMPLES_DRAFT4_SCHEMA_VALIDATION_NOT_LIVE_OR_RUNTIME_ORACLE",
        "fixture": "adapter/marketplace/tests/fixtures/official-responses.json",
        "fixture_sha256": hashlib.sha256((directory / "official-responses.json").read_bytes()).hexdigest(),
        "source_models": {case["source_path"]: case["source_sha256"] for case in fixture["cases"]},
        "whole_donor_absorption": False, "new_live_acquisition": False,
    }
    (ROOT / "research/commerce/sp-api-schema-proof.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
