#!/usr/bin/env python3
"""Contract probe of Google Trends' daily-trends RSS, read by ECDEV's GOOGLE_TRENDS_RSS feed source.

Usage: google_trends_rss_probe.py <ecdev_side binary> <capture dir> > result.json
Checks robots.txt for /trending/rss, then for each country feed: no guid, every <link> the feed
itself (so items are not posts), traffic bands of the form "N+", and that ECDEV's own reader
(xml_feed::google_trends_items) returns the same terms and bands as Python's standard parser.
Bodies stay in the capture dir outside the repository and are hashed here; only counts and
band strings are kept. API_CONTRACT_PROBE, not an ECDEV runtime capture.
"""
import hashlib, json, os, re, subprocess, sys, time, urllib.request
import xml.etree.ElementTree as ET

side, capdir = sys.argv[1], sys.argv[2]
os.makedirs(capdir, exist_ok=True)
HT = "{https://trends.google.com/trending/rss}"

def get(url):
    time.sleep(1.5)
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ECDEV"}), timeout=25) as r:
        return r.status, r.headers.get("Content-Type", ""), r.read()

s, _, robots = get("https://trends.google.com/robots.txt")
rules = [l.split(":", 1)[1].strip() for l in robots.decode().splitlines() if l.lower().startswith("disallow:")]
out = {"class": "API_CONTRACT_PROBE_NOT_ECDEV_RUNTIME_CAPTURE", "probed_at": int(time.time()),
       "robots": {"status": s, "disallow": rules, "trending_rss_disallowed": any("/trending/rss".startswith(r) for r in rules if r)},
       "terms": "Google Terms of Service apply; the feed is Google's published RSS subscription interface. No legal conclusion is drawn; whether a use fits the terms is the operator's decision.",
       "feeds": []}
for geo in ["JP", "US", "GB"]:
    url = f"https://trends.google.com/trending/rss?geo={geo}"
    status, ctype, body = get(url)
    h = hashlib.sha256(body).hexdigest()
    path = os.path.join(capdir, h + ".xml"); open(path, "wb").write(body)
    items = ET.fromstring(body).find("channel").findall("item")
    py = [(i.findtext("title"), i.findtext(HT + "approx_traffic")) for i in items]
    ec = json.loads(subprocess.run([side, "gtrends", path], capture_output=True, text=True).stdout)
    ecl = [(x["title"], x["approx_traffic"]) for x in ec["feed"]["items"]] if ec["ok"] else None
    out["feeds"].append({"geo": geo, "status": status, "content_type": ctype, "sha256": h, "items": len(items),
        "any_guid": any(i.find("guid") is not None for i in items),
        "every_link_is_the_feed": all(i.findtext("link") == url for i in items),
        "bands": sorted({b for _, b in py}), "bands_all_N_plus": all(re.fullmatch(r"\d+\+", b or "") for _, b in py),
        "ecdev_reader_agrees": ecl == py, "ecdev_error": None if ec["ok"] else ec["error"]})
print(json.dumps(out, indent=1, ensure_ascii=False))
