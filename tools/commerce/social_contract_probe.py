#!/usr/bin/env python3
"""Bounded probe of public social API contracts that ECDEV's social acquisition relies on.

Usage: social_contract_probe.py > result.json
This is an API-contract probe through the session's egress proxy, NOT an ECDEV runtime capture:
nothing it reads is stored as evidence or labelled LIVE. It checks, with the fewest requests:
cursor chaining, time-bounded queries and reply trees on Hacker News (Algolia) and Bluesky
(public AppView), and JSON Feed next_url. ECDEV user agent, 1.5 s between requests; bodies are
hashed, never kept.
"""
import hashlib, json, time, urllib.parse, urllib.request

UA = "ECDEV"
last = [0.0]
log = []

def get(url):
    wait = 1.5 - (time.time() - last[0])
    if wait > 0:
        time.sleep(wait)
    last[0] = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": UA}), timeout=25) as r:
            body, status = r.read(), r.status
    except urllib.error.HTTPError as e:
        body, status = e.read(), e.code
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    try:
        return status, json.loads(body)
    except Exception:
        return status, None

out = {}
HN = "https://hn.algolia.com/api/v1/"
q = "matcha"
s0, p0 = get(HN + "search_by_date?" + urllib.parse.urlencode({"query": q, "tags": "(story,comment)", "hitsPerPage": 50}))
s1, p1 = get(HN + "search_by_date?" + urllib.parse.urlencode({"query": q, "tags": "(story,comment)", "hitsPerPage": 50, "page": 1}))
ids0, ids1 = {h["objectID"] for h in p0["hits"]}, {h["objectID"] for h in p1["hits"]}
out["hn_paging"] = {"page0": {k: p0[k] for k in ("page", "nbPages", "nbHits", "hitsPerPage")}, "page1": {k: p1[k] for k in ("page", "nbPages", "nbHits")},
                    "overlap_ids": len(ids0 & ids1), "page0_time_range": [min(h["created_at_i"] for h in p0["hits"]), max(h["created_at_i"] for h in p0["hits"])],
                    "page1_time_range": [min(h["created_at_i"] for h in p1["hits"]), max(h["created_at_i"] for h in p1["hits"])],
                    "max_reachable_hits": p0["nbPages"] * p0["hitsPerPage"]}
now = int(time.time())
start, end = now - 86400, now
s2, p2 = get(HN + "search_by_date?" + urllib.parse.urlencode({"query": q, "tags": "(story,comment)", "hitsPerPage": 50, "numericFilters": f"created_at_i>{start},created_at_i<={end}"}))
out["hn_time_slice"] = {"requested": [start, end], "nbHits": p2["nbHits"], "nbPages": p2["nbPages"], "returned": len(p2["hits"]),
                        "all_within": all(start < h["created_at_i"] <= end for h in p2["hits"]), "exhaustive_flag": p2.get("exhaustive", p2.get("exhaustiveNbHits"))}
story = next((h for h in p0["hits"] if "story" in h.get("_tags", []) and (h.get("num_comments") or 0) > 0), None)
if story is None:
    st, sp = get(HN + "search?" + urllib.parse.urlencode({"query": q, "tags": "story", "numericFilters": "num_comments>3", "hitsPerPage": 1}))
    story = sp["hits"][0]
s3, tree = get(HN + "items/" + story["objectID"])
def walk(n, depth=0, acc=None):
    acc = acc if acc is not None else {"nodes": 0, "max_depth": 0, "missing_author": 0, "with_parent": 0, "fields": set()}
    acc["nodes"] += 1; acc["max_depth"] = max(acc["max_depth"], depth)
    acc["missing_author"] += n.get("author") is None
    acc["with_parent"] += n.get("parent_id") is not None
    acc["fields"] |= set(n.keys())
    for c in n.get("children", []):
        walk(c, depth + 1, acc)
    return acc
w = walk(tree)
out["hn_tree"] = {"story_id": story["objectID"], "story_num_comments": story.get("num_comments"), "tree_nodes": w["nodes"], "max_depth": w["max_depth"],
                  "nodes_without_author": w["missing_author"], "nodes_with_parent_id": w["with_parent"], "node_fields": sorted(w["fields"])}
BS = "https://api.bsky.app/xrpc/"  # public.api.bsky.app refuses searchPosts with 403
b0s, b0 = get(BS + "app.bsky.feed.searchPosts?" + urllib.parse.urlencode({"q": q, "sort": "latest", "limit": 25}))
out["bluesky_search"] = {"status": b0s, "posts": len((b0 or {}).get("posts", [])), "cursor": (b0 or {}).get("cursor"), "hitsTotal": (b0 or {}).get("hitsTotal")}
if b0s == 200 and b0.get("cursor"):
    b1s, b1 = get(BS + "app.bsky.feed.searchPosts?" + urllib.parse.urlencode({"q": q, "sort": "latest", "limit": 25, "cursor": b0["cursor"]}))
    u0 = {p["uri"] for p in b0["posts"]}; u1 = {p["uri"] for p in (b1 or {}).get("posts", [])}
    out["bluesky_search"]["page1"] = {"status": b1s, "posts": len(u1), "overlap": len(u0 & u1), "cursor": (b1 or {}).get("cursor")}
    since = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start)); until = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end))
    b2s, b2 = get(BS + "app.bsky.feed.searchPosts?" + urllib.parse.urlencode({"q": q, "sort": "latest", "limit": 25, "since": since, "until": until}))
    times = [p["record"].get("createdAt") for p in (b2 or {}).get("posts", [])]
    out["bluesky_time_slice"] = {"status": b2s, "requested": [since, until], "posts": len(times), "earliest": min(times) if times else None, "latest": max(times) if times else None,
                                 "all_within": all(since <= t <= until for t in times if t)}
    replied = next((p for p in b0["posts"] if (p.get("replyCount") or 0) > 0), None) or (b0["posts"][0] if b0["posts"] else None)
    if replied:
        t_s, t = get(BS + "app.bsky.feed.getPostThread?" + urllib.parse.urlencode({"uri": replied["uri"], "depth": 6, "parentHeight": 0}))
        def bwalk(n, depth=0, acc=None):
            acc = acc if acc is not None else {"nodes": 0, "max_depth": 0, "types": set(), "not_found": 0, "blocked": 0}
            acc["nodes"] += 1; acc["max_depth"] = max(acc["max_depth"], depth); acc["types"].add(n.get("$type"))
            acc["not_found"] += n.get("$type", "").endswith("notFoundPost"); acc["blocked"] += n.get("$type", "").endswith("blockedPost")
            for r in n.get("replies", []) or []:
                bwalk(r, depth + 1, acc)
            return acc
        bw = bwalk((t or {}).get("thread", {}))
        out["bluesky_thread"] = {"status": t_s, "root_replyCount": replied.get("replyCount"), "nodes": bw["nodes"], "max_depth": bw["max_depth"], "node_types": sorted(x for x in bw["types"] if x),
                                 "not_found": bw["not_found"], "blocked": bw["blocked"]}
jf_s, jf = get("https://www.jsonfeed.org/feed.json")
out["json_feed"] = {"status": jf_s, "version": (jf or {}).get("version"), "items": len((jf or {}).get("items", [])), "next_url": (jf or {}).get("next_url")}
print(json.dumps({"probe": "SOCIAL_API_CONTRACT_PROBE_NOT_ECDEV_RUNTIME", "captured_at": now, "results": out, "requests": log}, indent=1, default=sorted))
