#!/usr/bin/env python3
"""Benchmark ECDEV's native XML feed reader against Python's standard-library parser.

Usage: xml_feed_benchmark.py <ecdev_side binary> <capture dir> > result.json
Each public feed is fetched once through the session's egress proxy after its robots.txt is
checked with ECDEV's own robots evaluator; bodies are kept only in the capture dir (outside the
repository) and hashed here. The oracle is xml.etree.ElementTree with an item extraction written
independently of ECDEV's; dates are compared as instants (email.utils for RFC 822,
datetime.fromisoformat for RFC 3339). This is a BENCHMARK_MEASUREMENT, not an ECDEV runtime
capture: nothing here is stored as evidence or labelled LIVE.
"""
import datetime, email.utils, hashlib, json, os, subprocess, sys, time, urllib.parse, urllib.request
import xml.etree.ElementTree as ET

# Feeds whose sites refuse AI agents in robots.txt (NHK, Yahoo! News Japan, TechCrunch, The
# Verge) were removed after 2026-10-07; the gate below skips any that start refusing.
FEEDS = [
    "https://prtimes.jp/index.rdf",
    "https://feeds.bbci.co.uk/news/business/rss.xml",
    "https://www.retaildive.com/feeds/news/",
    "https://www.modernretail.co/feed/",
    "https://blog.rust-lang.org/feed.xml",
    "https://github.blog/feed/",
    "https://hnrss.org/frontpage",
    "https://www.nasa.gov/feed/",
    "https://www.itmedia.co.jp/rss/2.0/news_bursts.xml",
    "https://gigazine.net/news/rss_2.0/",
    "https://www.shopify.com/news/feed",
]
side, capdir = sys.argv[1], sys.argv[2]
os.makedirs(capdir, exist_ok=True)
last = [0.0]

def get(url):
    wait = 1.5 - (time.time() - last[0])
    if wait > 0:
        time.sleep(wait)
    last[0] = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ECDEV"}), timeout=25) as r:
            return r.status, r.read(), r.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        return e.code, e.read(), e.headers.get("Content-Type", "")
    except Exception as e:
        return None, str(e).encode(), ""

def local(tag):
    return tag.rsplit("}", 1)[-1]

def kid(e, name):
    for c in e:
        if local(c.tag) == name:
            return c
    return None

def txt(e, name):
    c = kid(e, name)
    t = (c.text or "").strip() if c is not None else ""
    return t or None

def instant(s):
    if not s:
        return None
    try:
        return int(email.utils.parsedate_to_datetime(s).timestamp())
    except Exception:
        pass
    try:
        return int(datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp())
    except Exception:
        return None

def oracle(body):
    root = ET.fromstring(body)
    kind = local(root.tag)
    out = []
    if kind == "rss":
        ch = kid(root, "channel")
        entries = [c for c in ch if local(c.tag) == "item"]
    elif kind == "RDF":
        entries = [c for c in root if local(c.tag) == "item"]
    elif kind == "feed":
        entries = [c for c in root if local(c.tag) == "entry"]
    else:
        return kind, None
    for e in entries:
        if kind == "feed":
            link = next((l.get("href") for l in e if local(l.tag) == "link" and l.get("rel") in (None, "alternate")), None)
            out.append({"id": txt(e, "id"), "url": link, "title": txt(e, "title"), "t": instant(txt(e, "published") or txt(e, "updated"))})
        else:
            about = e.get("{http://www.w3.org/1999/02/22-rdf-syntax-ns#}about")
            link = txt(e, "link")
            date = txt(e, "pubDate") or txt(e, "date")
            out.append({"id": txt(e, "guid") or about or link, "url": link, "title": txt(e, "title"), "t": instant(date)})
    return kind, out

results = []
for url in FEEDS:
    u = urllib.parse.urlsplit(url)
    rs, rb, _ = get(f"{u.scheme}://{u.netloc}/robots.txt")
    rpath = os.path.join(capdir, hashlib.sha256(rb).hexdigest() + ".robots")
    open(rpath, "wb").write(rb)
    allowed = True
    if rs == 200:
        verdict = json.loads(subprocess.run([side, "robots", rpath, url], capture_output=True, text=True).stdout)
        allowed = verdict["allowed"]
    row = {"feed": url, "robots_status": rs, "robots_allowed": allowed}
    if not allowed:
        row["result"] = "SKIPPED_ROBOTS"
        results.append(row)
        continue
    # Sites refusing AI agents (or AI input) are not read, as ECDEV's fetchers now enforce.
    if rs == 200:
        policy = json.loads(subprocess.run([side, "aipolicy", rpath, url], capture_output=True, text=True).stdout)
        if policy["refusal"]:
            row["result"] = "SKIPPED_AI_AGENT_REFUSAL"
            row["ai_agent_refusal"] = policy["refusal"]
            results.append(row)
            continue
    status, body, ctype = get(url)
    row.update({"status": status, "content_type": ctype, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    if status != 200:
        row["result"] = "FETCH_FAILED"
        results.append(row)
        continue
    path = os.path.join(capdir, row["sha256"] + ".xml")
    open(path, "wb").write(body)
    ecdev = json.loads(subprocess.run([side, "xmlfeed", path], capture_output=True, text=True).stdout)
    try:
        okind, oitems = oracle(body)
        row["oracle"] = {"root": okind, "items": None if oitems is None else len(oitems)}
    except Exception as e:
        okind, oitems = None, None
        row["oracle"] = {"error": type(e).__name__ + ": " + str(e)[:120]}
    if not ecdev["ok"]:
        row["ecdev"] = {"refused": ecdev["error"]}
        row["result"] = "BOTH_REFUSED" if oitems is None else "ECDEV_REFUSED_ORACLE_READ"
        results.append(row)
        continue
    items = ecdev["feed"]["items"]
    row["ecdev"] = {"kind": ecdev["feed"]["xml_kind"], "items": len(items), "items_without_parsable_date": ecdev["feed"]["items_without_parsable_date"]}
    if oitems is None:
        row["result"] = "ECDEV_READ_ORACLE_REFUSED"
        results.append(row)
        continue
    mism = []
    for i, (a, b) in enumerate(zip(items, oitems)):
        for k in ("id", "url", "title"):
            if a.get(k) != b[k]:
                mism.append({"item": i, "field": k, "ecdev": a.get(k), "oracle": b[k]})
        ta = instant(a.get("date_published"))
        if ta != b["t"]:
            mism.append({"item": i, "field": "instant", "ecdev": a.get("date_published"), "oracle": b["t"]})
    row["field_comparisons"] = 4 * min(len(items), len(oitems))
    row["mismatches"] = mism[:20]
    row["mismatch_count"] = len(mism)
    row["result"] = "AGREE" if not mism and len(items) == len(oitems) else "DISAGREE"
    results.append(row)

summary = {k: sum(1 for r in results if r["result"] == k) for k in sorted({r["result"] for r in results})}
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "measured_at": int(time.time()),
                  "oracle": "python xml.etree.ElementTree + independent extraction; instants via email.utils / datetime.fromisoformat",
                  "summary": summary, "feeds": results}, indent=1, ensure_ascii=False))
