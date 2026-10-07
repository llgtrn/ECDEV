"""Frozen request contracts of the SP-API operations ECDEV knows, from the pinned official models.

python sp_api_contracts.py <dir of model files at 3677bb9d> <out.json>
The directory holds each model as <model path with "/" replaced by "__">, fetched from
raw.githubusercontent.com/amzn/selling-partner-api-models/<COMMIT>/<model path>; every file must
match its git blob hash and SHA-256 at the pinned commit, which this script records. For each
operation: method, path template and every declared parameter (in, required, type, enum, array
item enum, collection format, maxItems) resolved through local $refs; for body parameters the
schema's required top-level properties. Nothing is executed; the models are read as data.
"""
import hashlib, json, pathlib, sys

src = pathlib.Path(sys.argv[1]).resolve(); out = pathlib.Path(sys.argv[2])
COMMIT = "3677bb9d96f4450e6843f1f8207005e925d5c867"
OPS = {
    "models/catalog-items-api-model/catalogItems_2022-04-01.json": ["getCatalogItem", "searchCatalogItems"],
    "models/product-pricing-api-model/productPricingV0.json": ["getItemOffers"],
    "models/product-fees-api-model/productFeesV0.json": ["getMyFeesEstimateForASIN"],
    "models/listings-items-api-model/listingsItems_2021-08-01.json": ["getListingsItem", "putListingsItem", "patchListingsItem", "deleteListingsItem"],
    "models/fba-inventory-api-model/fbaInventory.json": ["getInventorySummaries"],
    "models/sellers-api-model/sellers.json": ["getMarketplaceParticipations"],
    "models/product-type-definitions-api-model/definitionsProductTypes_2020-09-01.json": ["searchDefinitionsProductTypes", "getDefinitionsProductType"],
    "models/orders-api-model/ordersV0.json": ["getOrders", "getOrder", "getOrderBuyerInfo", "getOrderAddress", "getOrderItems", "getOrderItemsBuyerInfo", "updateShipmentStatus", "confirmShipment", "updateVerificationStatus"],
    "models/tokens-api-model/tokens_2021-03-01.json": ["createRestrictedDataToken"],
}

def resolve(doc, node):
    while isinstance(node, dict) and "$ref" in node:
        ref = node["$ref"]; assert ref.startswith("#/")
        target = doc
        for part in ref[2:].split("/"): target = target[part]
        node = target
    return node

contracts = []; sources = {}
for path, wanted in OPS.items():
    raw = (src / path.replace("/", "__")).read_bytes()
    sources[path] = {"blob_hash": hashlib.sha1(b"blob %d\0" % len(raw) + raw).hexdigest(), "sha256": hashlib.sha256(raw).hexdigest()}
    doc = json.loads(raw)
    found = set()
    for template, item in doc["paths"].items():
        shared = item.get("parameters", [])
        for method, op in item.items():
            if method not in ("get", "put", "post", "patch", "delete") or op.get("operationId") not in wanted: continue
            params = []
            for p in shared + op.get("parameters", []):
                p = resolve(doc, p)
                entry = {"name": p["name"], "in": p["in"], "required": bool(p.get("required", False))}
                for k in ("type", "enum", "collectionFormat", "maxItems"):
                    if k in p: entry[k] = p[k]
                if "items" in p:
                    items = resolve(doc, p["items"])
                    entry["items"] = {k: items[k] for k in ("type", "enum") if k in items}
                if p["in"] == "body":
                    schema = resolve(doc, p["schema"])
                    entry["body_required"] = schema.get("required", [])
                params.append(entry)
            contracts.append({"operation_id": op["operationId"], "method": method.upper(), "path": template, "model_path": path, "parameters": params})
            found.add(op["operationId"])
    assert found == set(wanted), (path, set(wanted) - found)
generator = pathlib.Path(__file__).read_bytes()
out.write_text(json.dumps({"donor_id": "amzn--selling-partner-api-models", "commit_sha": COMMIT, "license": "Apache-2.0",
    "oracle": "PINNED_OFFICIAL_MODELS_READ_AS_DATA", "source_models": sources,
    "generator_sha256": hashlib.sha256(generator).hexdigest(), "generator_source": generator.decode(),
    "contracts": sorted(contracts, key=lambda c: c["operation_id"])}, indent=1, ensure_ascii=False) + "\n")
print(len(contracts), "operation contracts")
