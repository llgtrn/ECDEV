"""Path/query-shape productivity and subtree yield on captured traces; offline exclusion-rule evaluation.
python -I scope.py <out.json> <trace.json>..."""
import json, sys, collections
out = sys.argv[1]
T = {}
for f in sys.argv[2:]:
    d = json.load(open(f)); T[d["site"]] = d
def yld(r): return r.get("new_identities", 0) + r.get("new_fields_on_known", 0) + r.get("new_listing_identities", 0)
res = {}
for site, d in T.items():
    P = [r for r in d["pages"] if r.get("fetched")]
    allrows = d["pages"]
    parent = {r["url"]: r["parent"] for r in P}
    shape = {r["url"]: r["shape"] for r in P}
    g = collections.defaultdict(lambda: collections.Counter())
    sub = collections.Counter(); subpages = collections.Counter()
    for r in P:
        s = r["shape"]; g[s]["fetched"] += 1
        g[s]["product_pages"] += r.get("role") == "PRODUCT"; g[s]["listing_pages"] += r.get("role") == "LISTING"
        g[s]["own_new_identities"] += r.get("new_identities", 0); g[s]["new_listing_identities"] += r.get("new_listing_identities", 0)
        g[s]["errors"] += r.get("state") != "HANDLED"
        g[s]["children_admitted"] += r.get("children_admitted", 0)
        y = yld(r)
        # credit every ancestor shape (once per shape per page chain) with the yield: subtree yield
        u = r["url"]; seen = set(); chain = set()
        while u is not None and u not in seen:
            seen.add(u)
            if u in shape: chain.add(shape[u])
            u = parent.get(u)
        for sh in chain:
            sub[sh] += y; subpages[sh] += 1
    refused = collections.Counter()
    for r in allrows:
        if r.get("reason", "") and str(r["reason"]).startswith("ROBOTS"): refused[r["shape"]] += 1
    groups = []
    for sh, c in g.items():
        groups.append({"shape": sh, **c, "page_yield": c["own_new_identities"] + c["new_listing_identities"], "subtree_yield": sub[sh], "subtree_pages": subpages[sh], "policy_refused_children_seen": 0})
    groups.sort(key=lambda x: -x["fetched"])
    # exclusion rule evaluation: exclude shape G (never fetch it nor anything beneath it)
    rules = []
    total_y = sum(yld(r) for r in P)
    for gr in groups:
        lost_pages = gr["subtree_pages"]; lost_y = gr["subtree_yield"]
        rules.append({"exclude_shape": gr["shape"], "pages_not_fetched": lost_pages, "pages_frac": round(lost_pages / len(P), 4), "yield_lost": lost_y, "yield_lost_frac": round(lost_y / total_y, 4) if total_y else None, "page_yield_of_group": gr["page_yield"], "subtree_yield_of_group": gr["subtree_yield"]})
    res[site] = {"fetched": len(P), "total_yield": total_y, "policy_refused_urls": dict(refused), "groups": groups[:25], "exclusion_rules": [r for r in rules if r["pages_frac"] >= 0.03][:25]}
json.dump(res, open(out, "w"), indent=1)
for site, v in res.items():
    print("==", site, "fetched", v["fetched"], "total_yield", v["total_yield"])
    for gr in v["groups"][:8]:
        print("  %-38s fetched=%-3d prod=%-3d list=%-3d pageY=%-5d subtreeY=%-5d" % (gr["shape"][:38], gr["fetched"], gr["product_pages"], gr["listing_pages"], gr["page_yield"], gr["subtree_yield"]))
