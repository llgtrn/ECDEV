"""Build a replayable per-page yield trace from one stored ECDEV crawl run.
python -I build_trace.py <engine root> <helper binary> <site> <out.json>
Reads the crawl database (fetch order, parent, depth, state) and each fetched page's stored raw capture,
re-extracts it with ECDEV's own extractor (helper `trace`) and records, per fetched page, the components of
information yield. Nothing is requested from the site."""
import json, sqlite3, subprocess, sys, urllib.parse, re, collections
root, helper, site, out = sys.argv[1:5]
db = sqlite3.connect(f"{root}/.ecdev-data/runtime/ecdev.sqlite")
urls = {r[0]: r for r in db.execute("select canonical_url,source_page,depth,state,payload,last_error,identity_hash from crawl_urls")}
by_hash = {r[6]: r[0] for r in urls.values()}
# fetch order: terminal events in event order
order = []
seen = set()
for _id, h, state, reason in db.execute("select id,identity_hash,state,reason from crawl_events order by id"):
    if state in ("HANDLED", "FAILED") and h not in seen:
        seen.add(h); order.append((h, state, reason))
children = collections.defaultdict(list)
for u, r in urls.items():
    if r[1]: children[r[1]].append(u)
SCALAR_SKIP = {"kind", "canonical_url", "price_status", "sales_status", "page_relation", "relation", "provenance", "price_source", "title_source", "variants"}
IDK = ["gtin14", "gtin13", "gtin12", "gtin8", "gtin", "ean", "upc", "jan", "isbn"]
def ident(p, page):
    for k in IDK:
        if p.get(k): return "gtin:" + str(p[k])
    if p.get("mpn"): return "mpn:" + str(p.get("brand") or "") + ":" + str(p["mpn"])
    host = urllib.parse.urlsplit(page).netloc
    if p.get("sku"): return "sku:" + host + ":" + str(p["sku"])
    if p.get("canonical_url"): return "url:" + p["canonical_url"]
    return "title:" + host + ":" + str(p.get("title"))
def shape(u):
    s = urllib.parse.urlsplit(u)
    segs = [x for x in s.path.split("/") if x]
    g = []
    for x in segs:
        if re.fullmatch(r"\d+", x): g.append("#")
        elif re.fullmatch(r"[0-9a-fA-F-]{8,}", x): g.append("#")
        elif re.search(r"\d", x) and len(x) > 6: g.append("~id")
        else: g.append(x if len(g) < 1 else "*")
    q = sorted({k for k, _ in urllib.parse.parse_qsl(s.query)})
    return "/" + "/".join(g) + ("?" + "&".join(q) if q else "")
rows = []
cum_ident = {}; cum_list = {}; cum_fields = set(); cum_assert = set(); vals = collections.defaultdict(set); conflicts = set()
for ordinal, (h, state, reason) in enumerate(order, 1):
    u = by_hash[h]; r = urls[u]
    row = {"ordinal": ordinal, "url": u, "parent": r[1], "depth": r[2], "shape": shape(u), "state": state, "reason": reason}
    kids = children.get(u, [])
    row["children_admitted"] = len(kids)
    row["children_refused_by_policy"] = sum(1 for k in kids if (urls[k][5] or "").startswith("ROBOTS"))
    if state != "HANDLED":
        row["fetched"] = reason not in ("ROBOTS_DENIED_OR_UNKNOWN", "ROBOTS_REFUSES_AI_AGENTS")
        rows.append(row); continue
    row["fetched"] = True
    pl = json.loads(r[4]); cap = pl["capture"]
    ch = cap["result"].get("content_hash") or next((o.get("raw_hash") for o in cap.get("observations", []) if o.get("raw_hash")), None)
    import os
    if not ch or not os.path.exists(f"{root}/.ecdev-data/raw/{ch}.html"):
        row["role"] = "NO_RAW_CAPTURE"; row["result_keys"] = sorted(cap["result"])[:8]; rows.append(row); continue
    t = json.loads(subprocess.check_output([helper, "trace", f"{root}/.ecdev-data/raw/{ch}.html", "text/html", u]))
    row["raw_capture_sha256"] = ch
    prods = t.get("products", []) if t.get("ok") else []
    links = t.get("links", []) if t.get("ok") else []
    own = [p for p in prods if (p.get("page_relation") or {}).get("relation", p.get("relation")) in ("PAGE_SUBJECT", "PAGE_SUBJECT_VARIANT", "NOT_STATED", None)]
    row["links_out"] = len(set(links)); row["products_extracted"] = len(prods); row["own_products"] = len(own)
    row["role"] = "PRODUCT" if own else ("LISTING" if prods else "OTHER")
    new_id = new_idb = enrich_fields = new_assert = new_conf = 0
    touched = set()
    for p in own:
        i = ident(p, u)
        is_new = i not in cum_ident
        if is_new:
            cum_ident[i] = {"first": ordinal, "idb": i.startswith(("gtin:", "mpn:"))}
            new_id += 1; new_idb += cum_ident[i]["idb"]
        touched.add(i)
        for k, v in p.items():
            if k in SCALAR_SKIP or v is None or isinstance(v, (dict, list)) or v == "": continue
            if k not in ("title", "brand", "price_minor", "currency", "availability", "sku", "mpn", "ean", "category", "color", "description", "image", "rating", "review_count", "seller", "weight_g") and not k.startswith("gtin"): continue
            if (i, k) not in cum_fields:
                cum_fields.add((i, k))
                if not is_new: enrich_fields += 1
            a = (i, k, json.dumps(v, sort_keys=True))
            if a not in cum_assert:
                cum_assert.add(a); new_assert += 1
                vals[(i, k)].add(a[2])
                if len(vals[(i, k)]) == 2: conflicts.add((i, k)); new_conf += 1
    others = [p for p in prods if p not in own]
    new_list = 0
    for p in others:
        i = ident(p, p.get("canonical_url") or u)
        if i not in cum_ident and i not in cum_list:
            cum_list[i] = ordinal; new_list += 1
    row["new_listing_identities"] = new_list; row["cum_listing_identities"] = len(cum_list)
    row["new_identities"] = new_id; row["new_identifier_bearing"] = new_idb
    row["new_fields_on_known"] = enrich_fields; row["new_assertions"] = new_assert; row["new_conflicts"] = new_conf
    row["enriched_known"] = (not new_id) and (enrich_fields > 0)
    row["cum_identities"] = len(cum_ident); row["cum_fields"] = len(cum_fields); row["cum_assertions"] = len(cum_assert); row["cum_conflicts"] = len(conflicts)
    rows.append(row)
json.dump({"site": site, "pages": rows, "identities": cum_ident}, open(out, "w"), indent=0)
fetched = [r for r in rows if r.get("fetched")]
print(site, "fetched", len(fetched), "handled", sum(r["state"] == "HANDLED" for r in rows), "identities", len(cum_ident), "roles", dict(collections.Counter(r.get("role") for r in rows if r["state"] == "HANDLED")))
