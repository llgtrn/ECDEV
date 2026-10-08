"""Offline replay of early-stop and path-scope policies on captured per-page yield traces.
python -I replay.py <out.json> <trace.json>...
No network. The full bounded trace is the baseline, never the catalogue total."""
import json, sys, collections, itertools
out = sys.argv[1]
T = {}
for f in sys.argv[2:]:
    d = json.load(open(f)); T[d["site"]] = d
def pages(d): return [r for r in d["pages"] if r.get("fetched")]
def low(r): return r.get("role") is None or (r.get("new_identities", 0) + r.get("new_fields_on_known", 0) + r.get("new_listing_identities", 0)) == 0
def totals(rows):
    ids = set(); idb = 0; fields = 0; assertions = 0; conflicts = 0
    return {"identities": sum(r.get("new_identities", 0) for r in rows), "listing_identities": sum(r.get("new_listing_identities", 0) for r in rows), "identifier_bearing": sum(r.get("new_identifier_bearing", 0) for r in rows),
            "enrichment_fields": sum(r.get("new_fields_on_known", 0) for r in rows), "assertions": sum(r.get("new_assertions", 0) for r in rows), "conflicts": sum(r.get("new_conflicts", 0) for r in rows)}
def metrics(d, kept_ordinals):
    P = pages(d); kept = [r for r in P if r["ordinal"] in kept_ordinals]
    full, got = totals(P), totals(kept)
    n = len(P); k = len(kept)
    last_q = P[int(n * 0.75):]
    late_full = totals(last_q)["identities"]; late_got = totals([r for r in last_q if r["ordinal"] in kept_ordinals])["identities"]
    m = {"pages": n, "pages_kept": k, "saved_pages": n - k, "saved_frac": round((n - k) / n, 4) if n else 0.0}
    for key in full:
        m[key + "_full"] = full[key]; m[key + "_kept"] = got[key]
        m[key + "_missed_frac"] = round(1 - got[key] / full[key], 4) if full[key] else None
    m["late_identities_full"] = late_full; m["late_identities_lost"] = late_full - late_got
    return m
def consec(d, N):
    """stop the whole crawl after N consecutive fetched pages with no new identity and no new field"""
    kept = set(); run = 0
    for r in pages(d):
        kept.add(r["ordinal"])
        run = run + 1 if low(r) else 0
        if run >= N: break
    return kept
def window(d, W, tau):
    """stop when new identities in the last W fetched pages <= tau (after at least W pages)"""
    kept = set(); hist = []
    for r in pages(d):
        kept.add(r["ordinal"]); hist.append(r.get("new_identities", 0) + r.get("new_listing_identities", 0))
        if len(hist) >= W and sum(hist[-W:]) <= tau: break
    return kept
def region(d, N, minpages=0):
    """per path-shape: after N consecutive zero-yield pages in that shape, skip the shape's later pages and their descendants"""
    P = pages(d); kept = set(); run = collections.Counter(); closed = set(); skipped_urls = set()
    for r in P:
        if r["shape"] in closed or r["parent"] in skipped_urls:
            skipped_urls.add(r["url"]); continue
        kept.add(r["ordinal"])
        run[r["shape"]] = run[r["shape"]] + 1 if low(r) else 0
        if run[r["shape"]] >= N: closed.add(r["shape"])
    return kept
def both(d, N, W):
    kept = set(); run = 0; hist = []
    for r in pages(d):
        kept.add(r["ordinal"]); hist.append(r.get("new_identities", 0) + r.get("new_listing_identities", 0))
        run = run + 1 if low(r) else 0
        if run >= N and len(hist) >= W and sum(hist[-W:]) == 0: break
    return kept
def subtree_adaptive(d, k):
    """per path-shape, online: close a shape once k of its pages were fetched and neither they nor any page
    below them has yielded a new identity or field so far (subtree yield, not page yield); skip its later pages
    and their descendants."""
    P = pages(d); kept = set(); fetched = collections.Counter(); sub = collections.Counter(); closed = set(); skipped = set()
    shape_of = {r["url"]: r["shape"] for r in P}; parent = {r["url"]: r["parent"] for r in P}
    for r in P:
        if r["shape"] in closed or r["parent"] in skipped:
            skipped.add(r["url"]); continue
        kept.add(r["ordinal"]); fetched[r["shape"]] += 1
        y = r.get("new_identities", 0) + r.get("new_fields_on_known", 0) + r.get("new_listing_identities", 0)
        if y:
            u = r["url"]; seen = set()
            while u is not None and u not in seen:
                seen.add(u)
                if u in shape_of: sub[shape_of[u]] += y
                u = parent.get(u)
        for sh in list(fetched):
            if sh not in closed and fetched[sh] >= k and sub[sh] == 0: closed.add(sh)
    return kept
GRID = [("consecutive_low", {"N": N}, lambda d, p: consec(d, p["N"])) for N in (3, 5, 8, 12, 20, 30)] + \
       [("rolling_identity_window", {"W": W, "tau": 0}, lambda d, p: window(d, p["W"], p["tau"])) for W in (10, 20, 30, 40)] + \
       [("region_consecutive_low", {"N": N}, lambda d, p: region(d, p["N"])) for N in (2, 3, 5, 8, 12)] + \
       [("region_subtree_adaptive", {"k": k}, lambda d, p: subtree_adaptive(d, p["k"])) for k in (2, 3, 5, 8, 12)] + \
       [("identity_and_enrichment_low", {"N": N, "W": W}, lambda d, p: both(d, p["N"], p["W"])) for N in (5, 10) for W in (20, 40)]
res = {}
for site, d in T.items():
    res[site] = {"trace": {"fetched_pages": len(pages(d)), **totals(pages(d))}, "policies": []}
    for name, params, fn in GRID:
        kept = fn(d, params)
        res[site]["policies"].append({"policy": name, "params": params, **metrics(d, kept)})
# leave-one-catalogue-out: pick, on the other sites with identities, the policy with the largest mean saved fraction
# whose worst missed-identity fraction is <= EPS; evaluate it on the held-out site.
EPS = 0.02
def key(p): return (p["policy"], json.dumps(p["params"], sort_keys=True))
loo = []
sites = [s for s in T if res[s]["trace"]["identities"] + res[s]["trace"]["listing_identities"] > 0]
for held in T:
    train = [s for s in sites if s != held]
    cand = collections.defaultdict(list)
    for s in train:
        for p in res[s]["policies"]: cand[key(p)].append(p)
    best = None
    for k, ps in cand.items():
        worst = max(max((p["identities_missed_frac"] or 0), (p["listing_identities_missed_frac"] or 0)) for p in ps)
        mean_saved = sum(p["saved_frac"] for p in ps) / len(ps)
        if worst <= EPS and (best is None or mean_saved > best[0]): best = (mean_saved, k, worst)
    row = {"held_out": held, "train_sites": train, "eps_max_missed_identity_fraction": EPS}
    if best is None:
        row["chosen"] = None; row["note"] = "no policy kept missed identities within EPS on every training site"
    else:
        k = best[1]; row["chosen"] = {"policy": k[0], "params": json.loads(k[1]), "train_mean_saved": round(best[0], 4), "train_worst_missed": best[2]}
        hp = next(p for p in res[held]["policies"] if key(p) == k)
        row["held_out_result"] = hp
    loo.append(row)
json.dump({"sites": res, "leave_one_catalogue_out": loo, "grid_size": len(GRID)}, open(out, "w"), indent=1)
for s, v in res.items(): print(s, v["trace"])
print("--- no-lose policies per site (missed identities == 0 and saved>0):")
for s, v in res.items():
    ok = [(p["policy"], p["params"], p["saved_frac"]) for p in v["policies"] if (p["identities_missed_frac"] in (0, 0.0, None)) and (p["listing_identities_missed_frac"] in (0, 0.0, None)) and p["saved_frac"] > 0]
    print(s, len(ok), sorted(ok, key=lambda x: -x[2])[:3])
print("--- LOO:")
for r in loo: print(r["held_out"], r.get("chosen"), (r.get("held_out_result") or {}).get("saved_frac"), (r.get("held_out_result") or {}).get("identities_missed_frac"))
