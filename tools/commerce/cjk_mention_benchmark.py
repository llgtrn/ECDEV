#!/usr/bin/env python3
"""Measure ECDEV's query-mention rule on real Japanese feed text, before and after CJK support.

Usage: cjk_mention_benchmark.py <ecdev_side binary> <dir of captured feed XML> > result.json
The texts are item titles and descriptions of public feeds captured by xml_feed_benchmark.py
(bodies stay outside the repository; their hashes are listed). The reference for a pure-CJK
query is verbatim substring presence after full-width folding and lowercasing, which is what a
reader would call a mention; mixed queries are reported without a reference. A
BENCHMARK_MEASUREMENT, not an ECDEV runtime capture.
"""
import hashlib, json, os, subprocess, sys, tempfile, unicodedata
import xml.etree.ElementTree as ET

side, capdir = sys.argv[1], sys.argv[2]
QUERIES = ["抹茶", "新発売", "値上げ", "円安", "発売", "販売", "限定", "セール", "コーヒー", "キャンペーン",
           "スマホ", "人気", "茶", "AI", "iPhone", "Amazon セール", "新商品 発売"]

def local(t):
    return t.rsplit("}", 1)[-1]

texts, sources = [], []
for name in sorted(os.listdir(capdir)):
    if not name.endswith(".xml"):
        continue
    body = open(os.path.join(capdir, name), "rb").read()
    try:
        root = ET.fromstring(body)
    except ET.ParseError:
        continue
    items = [e for e in root.iter() if local(e.tag) in ("item", "entry")]
    jp = []
    for e in items:
        parts = [(c.text or "") for c in e if local(c.tag) in ("title", "description", "summary")]
        t = " ".join(p.strip() for p in parts if p)
        if any("぀" <= ch <= "ヿ" or "一" <= ch <= "鿿" for ch in t):
            jp.append(t)
    if jp:
        sources.append({"capture_sha256": name[:-4], "japanese_items": len(jp)})
        texts.extend(jp)

def fold(s):
    return "".join(chr(ord(c) - 0xFEE0) if 0xFF01 <= ord(c) <= 0xFF5E else c for c in s).lower()

def is_cjk(c):
    return "぀" <= c <= "ヿ" or "㐀" <= c <= "鿿"

with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
    json.dump({"queries": QUERIES, "texts": texts}, f, ensure_ascii=False)
out = json.loads(subprocess.run([side, "mentions", f.name], capture_output=True, text=True).stdout)
os.unlink(f.name)
rows = []
for q in QUERIES:
    r = out[q]
    row = {"query": q, "query_valid_before": r["query_valid_before"], "query_valid_now": r["query_valid_now"],
           "matches_before": len(r["before"]), "matches_now": len(r["now"])}
    if all(is_cjk(c) for c in q):
        ref = {i for i, t in enumerate(texts) if fold(q) in fold(t)}
        now = set(r["now"])
        row.update({"reference_substring_matches": len(ref), "now_true_positive": len(now & ref),
                    "now_false_positive": len(now - ref), "now_missed": len(ref - now), "before_true_positive": len(set(r["before"]) & ref)})
    rows.append(row)
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "texts": len(texts), "sources": sources,
                  "reference": "verbatim substring after full-width folding and lowercasing (pure-CJK queries only)", "queries": rows},
                 indent=1, ensure_ascii=False))
