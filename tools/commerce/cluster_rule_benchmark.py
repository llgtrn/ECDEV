#!/usr/bin/env python3
"""Clustering rules measured against Hacker News story membership.

Usage: cluster_rule_benchmark.py <dir holding clu-<query>.json discover outputs and engine roots>
Posts come from live ecdev.trend.discover runs (queries rust, python, apple, coffee); each post's
story is read from its own raw capture at its raw locator (story_id, or the story's own id). Two
posts of the same story are a positive pair. This is a weak, structural label: comments under one
story can digress and different stories can share a topic. No model labels are used. Each rule is
run through ECDEV's own social::clusters_with (helper mode clusterrules); pairwise precision and
recall are reported. BENCHMARK_MEASUREMENT on captures kept outside the repository.
"""
S = sys.argv[1]
import json, collections, itertools, subprocess, sys
S = sys.argv[1]
posts, labels, seen = [], {}, set()
for q in ["rust", "python", "apple", "coffee"]:
    root = f"{S}/clu-{q}" if q != "coffee" else f"{S}/clu-root"
    d = json.load(open(f"{S}/clu-{q}.json" if q != "coffee" else f"{S}/clu.json"))
    for p in d["captured_posts"]:
        if p["native_id"] in seen:
            continue
        raw = json.load(open(f"{root}/.ecdev-data/runtime/social-captures/{p['raw_hash']}.raw"))
        node = raw
        for part in p["raw_locator"].strip("/").split("/"):
            node = node[int(part)] if isinstance(node, list) else node[part]
        seen.add(p["native_id"])
        posts.append(p)
        labels[f"{p['platform']}:{p['native_id']}"] = str(node.get("story_id") or node.get("objectID"))
rules = []
for overlap in (False, True):
    for informative in (False, True):
        for sim in (0.1, 0.15, 0.25, 0.4):
            for shared in (2, 3):
                rules.append({"min_similarity": sim, "min_shared": shared, "overlap": overlap, "informative_only": informative})
json.dump({"posts": posts, "rules": rules}, open(f"{S}/clu-in.json", "w"))
out = json.loads(subprocess.run(["./target/release/ecdev_side", "clusterrules", f"{S}/clu-in.json"], capture_output=True, text=True).stdout)
pos = {frozenset(p) for p in itertools.combinations(labels, 2) if labels[p[0]] == labels[p[1]]}
rows = []
for r in out:
    co = set()
    sizes = []
    for c in r["clusters"]:
        sizes.append(len(c))
        for a, b in itertools.combinations(c, 2):
            co.add(frozenset((a, b)))
    tp = len(co & pos)
    prec = tp / len(co) if co else None
    rec = tp / len(pos)
    f1 = 2 * prec * rec / (prec + rec) if prec and rec else 0.0
    rows.append({**r["rule"], "clusters": len(sizes), "multi_post_clusters": sum(1 for s in sizes if s > 1), "co_clustered_pairs": len(co), "precision": prec, "recall": rec, "f1": f1})
print(json.dumps({"posts": len(posts), "stories": len(set(labels.values())), "positive_pairs": len(pos), "rows": rows}))
