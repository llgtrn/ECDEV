#!/usr/bin/env python3
"""How much of Algolia's Hacker News search result is the query word, with and without its
default typo tolerance and prefix matching.

Usage: hn_exact_match_probe.py > result.json
For each query, the first page of 50 hits per setting, counting hits whose title or text contains
the query as a word start. API_CONTRACT_PROBE, not an ECDEV runtime capture; bodies are not kept.
"""
import json, re, time, urllib.parse, urllib.request

SETTINGS = {"default": {}, "typo_off": {"typoTolerance": "false"}, "typo_off_prefix_none": {"typoTolerance": "false", "queryType": "prefixNone"}}
out = {"class": "API_CONTRACT_PROBE_NOT_ECDEV_RUNTIME_CAPTURE", "probed_at": int(time.time()), "queries": {}}
for query in ["matcha", "whisk", "kettle"]:
    out["queries"][query] = {}
    for name, extra in SETTINGS.items():
        u = "https://hn.algolia.com/api/v1/search_by_date?" + urllib.parse.urlencode({"query": query, "tags": "(story,comment)", "hitsPerPage": 50, **extra})
        time.sleep(1.5)
        d = json.load(urllib.request.urlopen(urllib.request.Request(u, headers={"User-Agent": "ECDEV"}), timeout=25))
        lit = sum(bool(re.search(r"\b" + query, " ".join(str(h.get(k) or "") for k in ("title", "comment_text", "story_text")).lower())) for h in d["hits"])
        out["queries"][query][name] = {"nbHits": d["nbHits"], "page_hits": len(d["hits"]), "hits_containing_the_word": lit}
print(json.dumps(out, indent=1))
