#!/usr/bin/env python3
"""Bounded benchmark: how often does ECDEV's strict document decoder refuse real pages, and what
would statistical detection (charset_normalizer, as httpx uses) have guessed?

Usage: charset_benchmark.py <out dir> <ecdev helper binary> <home URL>... > result.json
ECDEV's own robots evaluator decides every URL (helper "robots"); ECDEV's own decoder reads every
body (helper "decode"). ECDEV user agent, 1.5 s between requests, the home page plus up to four
pages from the first uncompressed sitemap robots.txt declares (one index level followed), 4 MiB per body. Bodies are hashed,
never kept.
"""
import hashlib, json, re, subprocess, sys, time, urllib.parse, urllib.request, pathlib
import charset_normalizer

out, helper, homes = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3:]
out.mkdir(parents=True, exist_ok=True)
UA, MAX = "ECDEV", 4 * 1024 * 1024
last, log = [0.0], []

def get(url):
    wait = 1.5 - (time.time() - last[0])
    if wait > 0:
        time.sleep(wait)
    last[0] = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": UA}), timeout=20) as r:
            body, status, ctype = r.read(MAX + 1), r.status, r.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        body, status, ctype = b"", e.code, ""
    except Exception as e:
        body, status, ctype = b"", None, ""
        log.append({"url": url, "error": type(e).__name__})
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    return status, body, ctype

def allowed(robots_file, url):
    return json.loads(subprocess.check_output([helper, "robots", robots_file, url]))["allowed"]

def decode(body, ctype):
    f = out / "body.bin"; f.write_bytes(body)
    return json.loads(subprocess.check_output([helper, "decode", str(f), ctype or ""]))

results = []
for home in homes:
    origin = urllib.parse.urlsplit(home)._replace(path="", query="", fragment="").geturl()
    host = urllib.parse.urlsplit(origin).netloc
    s, robots, _ = get(origin + "/robots.txt")
    rf = out / f"robots-{host}.txt"
    rf.write_bytes(robots if s == 200 else b"")
    robots_arg = str(rf) if s == 200 else "-"
    row = {"site": origin, "robots_status": s, "pages": []}
    if s not in (200, 404):
        row["state"] = "ROBOTS_UNKNOWN"; results.append(row); continue
    urls = [home]
    text = robots.decode("utf-8", "replace") if s == 200 else ""
    queue = [l.split(":", 1)[1].strip() for l in text.splitlines() if l.lower().startswith("sitemap:")][:2]
    fetched = 0
    while queue and fetched < 3:
        sm = queue.pop(0)
        if sm.endswith(".gz") or not allowed(robots_arg, sm):
            continue
        st, body, _ = get(sm); fetched += 1
        locs = [u.replace("&amp;", "&") for u in re.findall(r"<loc>\s*(?:<!\[CDATA\[)?\s*(.*?)\s*(?:\]\]>)?\s*</loc>", body.decode("utf-8", "replace"), re.S)]
        if b"<sitemapindex" in body:
            # One level of index: its first uncompressed child sitemap.
            queue = [u for u in locs if not u.endswith(".gz")][:1] + queue
            continue
        pages = [u for u in locs if urllib.parse.urlsplit(u).netloc == host]
        if pages:
            urls += pages[:4]; break
    for u in urls:
        if not allowed(robots_arg, u):
            row["pages"].append({"url": u, "state": "ROBOTS_DENIED_BY_ECDEV"}); continue
        st, body, ctype = get(u)
        if st != 200 or not body:
            row["pages"].append({"url": u, "status": st, "state": "NOT_200"}); continue
        d = decode(body, ctype)
        best = charset_normalizer.from_bytes(body[:200_000]).best()
        try:
            body.decode("utf-8"); valid_utf8 = True
        except UnicodeDecodeError:
            valid_utf8 = False
        row["pages"].append({"url": u, "status": st, "content_type": ctype, "ecdev_ok": d["ok"], "ecdev_error": d.get("error"),
                             "ecdev_encoding": (d.get("recipe") or {}).get("encoding"), "ecdev_source": (d.get("recipe") or {}).get("source"),
                             "valid_utf8": valid_utf8, "detector_guess": best.encoding if best else None})
    row["state"] = "SAMPLED"
    results.append(row)
print(json.dumps({"results": results, "requests": log}, indent=1))
