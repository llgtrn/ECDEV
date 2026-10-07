#!/usr/bin/env python3
"""How often Japanese feed titles carry bracketed spans (「」『』【】), and which recur.

Usage: bracket_span_census.py <dir of captured feed XML> > result.json
Counts only; no precision is claimed, because no labelled product-name data exists. A span is
a candidate the operator verifies, never a name. BENCHMARK_MEASUREMENT on captures kept outside
the repository (hashes listed), not an ECDEV runtime capture.
"""
import collections, json, os, re, sys
import xml.etree.ElementTree as ET

d = sys.argv[1]
local = lambda t: t.rsplit("}", 1)[-1]
total = with_span = 0
spans = collections.Counter()
captures = []
for name in sorted(os.listdir(d)):
    if not name.endswith(".xml"):
        continue
    try:
        root = ET.parse(os.path.join(d, name)).getroot()
    except ET.ParseError:
        continue
    n = 0
    for e in root.iter():
        if local(e.tag) not in ("item", "entry"):
            continue
        t = " ".join((c.text or "") for c in e if local(c.tag) == "title")
        if not re.search("[぀-ヿ一-鿿]", t):
            continue
        n += 1
        found = set(re.findall("[「『【]([^」』】]{2,30})[」』】]", t))
        with_span += bool(found)
        spans.update(found)
    if n:
        captures.append({"capture_sha256": name[:-4], "japanese_titles": n})
    total += n
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "captures": captures,
                  "japanese_titles": total, "titles_with_bracketed_span": with_span, "distinct_spans": len(spans),
                  "spans_in_two_or_more_titles": sorted(s for s, c in spans.items() if c > 1),
                  "precision": "NOT_MEASURED_NO_LABELLED_PRODUCT_NAMES",
                  "reading": "Spans mix product and service names with quotations and slogans; ECDEV keeps only spans recurring in two posts by two authors, labelled CANDIDATE_PRODUCT_PHRASE_UNVERIFIED"},
                 indent=1, ensure_ascii=False))
