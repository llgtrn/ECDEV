#!/usr/bin/env python3
"""Words common to unrelated topics, measured on live captures rather than chosen by hand.

Usage: background_common_words.py [--japanese] [--min-topics N] <dir> <topic>=<discover output json>... > record.json
--min-topics N: common means at least 1% of a topic's posts in N topics (default: every topic).
Japanese samples of ~250 posts and a wide vocabulary reach no word in every topic, so the
Japanese record states the weaker criterion it used.
A word is common when it appears in at least 1% of the posts of every topic sample: such a word
says nothing about any one product, so it cannot refine a product query. Words are lowercase
Latin runs with links removed; with --japanese, script words of unspaced CJK text exactly as
domain/commerce/src/social.rs script_words reads them (runs of one script: katakana and kanji
of two or more characters, hiragana dropped), with links, @handles and bare domains removed. BENCHMARK_MEASUREMENT on captures kept outside the repository.
"""
import collections, json, re, sys

THRESHOLD = 0.01
args = sys.argv[1:]
japanese = "--japanese" in args
min_topics = None
if "--min-topics" in args:
    i = args.index("--min-topics")
    min_topics = int(args[i + 1])
    del args[i : i + 2]
argv = [a for a in args if a != "--japanese"]
base = argv[0]
samples = dict(a.split("=", 1) for a in argv[1:])


def script(c):
    o = ord(c)
    if 0x30A0 <= o <= 0x30FF or 0x31F0 <= o <= 0x31FF or 0xFF66 <= o <= 0xFF9F:
        return "K"
    if o in (0x3005, 0x3007) or 0x3400 <= o <= 0x4DBF or 0x4E00 <= o <= 0x9FFF or 0xF900 <= o <= 0xFAFF or 0x20000 <= o <= 0x2FFFF:
        return "H"
    if 0x3040 <= o <= 0x309F:
        return "I"
    return "O"


def script_words(text):
    text = " ".join(w for w in text.split() if "://" not in w and not w.startswith(("www.", "@")))
    out, cur, kind = [], "", "O"
    for c in text:
        k = script(c)
        if k != kind and cur:
            if kind in "KH" and len(cur) >= 2:
                out.append(cur)
            cur = ""
        kind = k
        cur = cur + c if k in "KH" else ""
        if k not in "KH":
            kind = k
    if cur and kind in "KH" and len(cur) >= 2:
        out.append(cur)
    return out

df, sizes = {}, {}
for topic, name in samples.items():
    posts = json.load(open(f"{base}/{name}"))["captured_posts"]
    counts = collections.Counter()
    for p in posts:
        if japanese:
            counts.update(set(script_words(p["text"])))
        else:
            counts.update(set(re.findall(r"[a-z][a-z']+", re.sub(r"https?://\S+", "", p["text"].lower()))))
    sizes[topic] = len(posts)
    df[topic] = {w: n / len(posts) for w, n in counts.items()}
need = min_topics or len(df)
vocabulary = set().union(*[set(d) for d in df.values()])
common = sorted(w for w in vocabulary if sum(1 for t in df if df[t].get(w, 0) >= THRESHOLD) >= need)
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "threshold_share_of_posts": THRESHOLD, "topics_required": need, "of_topics": len(df),
                  "topics": sizes, "posts": sum(sizes.values()), "common_words": common,
                  "document_frequency": {w: {t: round(df[t].get(w, 0), 4) for t in df} for w in common}}, indent=1))
