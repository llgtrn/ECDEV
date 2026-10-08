#!/usr/bin/env python3
"""Words common to unrelated topics, measured on live captures rather than chosen by hand.

Usage: background_common_words.py <dir> <topic>=<discover output json>... > record.json
A word is common when it appears in at least 1% of the posts of every topic sample: such a word
says nothing about any one product, so it cannot refine a product query. Words are lowercase
Latin runs with links removed. BENCHMARK_MEASUREMENT on captures kept outside the repository.
"""
import collections, json, re, sys

THRESHOLD = 0.01
base = sys.argv[1]
samples = dict(a.split("=", 1) for a in sys.argv[2:])
df, sizes = {}, {}
for topic, name in samples.items():
    posts = json.load(open(f"{base}/{name}"))["captured_posts"]
    counts = collections.Counter()
    for p in posts:
        counts.update(set(re.findall(r"[a-z][a-z']+", re.sub(r"https?://\S+", "", p["text"].lower()))))
    sizes[topic] = len(posts)
    df[topic] = {w: n / len(posts) for w, n in counts.items()}
first = next(iter(df.values()))
common = sorted(w for w in first if all(df[t].get(w, 0) >= THRESHOLD for t in df))
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "threshold_share_of_posts_in_every_topic": THRESHOLD,
                  "topics": sizes, "posts": sum(sizes.values()), "common_words": common,
                  "document_frequency": {w: {t: round(df[t][w], 4) for t in df} for w in common}}, indent=1))
