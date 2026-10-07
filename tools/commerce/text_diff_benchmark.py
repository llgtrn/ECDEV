#!/usr/bin/env python3
"""Bounded benchmark: how often does a page that states no product data change between two
captures minutes apart, by raw bytes, by ECDEV's visible text, and by changed lines?

Usage: text_diff_benchmark.py <out dir> <ecdev helper> <gap seconds> <home URL>... > result.json
Pages come from each site's own robots-declared sitemaps (pages sitemaps first); ECDEV's robots
evaluator decides every URL; ECDEV's page_text::visible_lines extracts text. Two passes over the
same URLs, <gap seconds> apart; ECDEV user agent, 1.5 s between requests, 4 MiB per body; bodies
are hashed and, for changed lines only, the differing lines are kept.
"""
import difflib, hashlib, json, re, subprocess, sys, time, urllib.parse, urllib.request, pathlib

out, helper, gap, homes = pathlib.Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), sys.argv[4:]
out.mkdir(parents=True, exist_ok=True)
UA, MAX, PER_SITE = "ECDEV", 4 * 1024 * 1024, 3
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
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    return status, body, ctype

def allowed(robots_file, url):
    return json.loads(subprocess.check_output([helper, "robots", robots_file, url]))["allowed"]

def lines(body):
    f = out / "page.html"; f.write_bytes(body)
    return json.loads(subprocess.check_output([helper, "text", str(f)]))

targets = []
for home in homes:
    origin = urllib.parse.urlsplit(home)._replace(path="", query="", fragment="").geturl()
    host = urllib.parse.urlsplit(origin).netloc
    s, robots, _ = get(origin + "/robots.txt")
    rf = out / f"robots-{host}.txt"; rf.write_bytes(robots if s == 200 else b"")
    if s not in (200, 404):
        continue
    ra = str(rf) if s == 200 else "-"
    maps = [l.split(":", 1)[1].strip() for l in robots.decode("utf-8", "replace").splitlines() if l.lower().startswith("sitemap:")]
    queue, fetched, pages = maps[:2], 0, []
    while queue and fetched < 3 and not pages:
        sm = queue.pop(0)
        if sm.endswith(".gz") or not allowed(ra, sm):
            continue
        st, body, _ = get(sm); fetched += 1
        locs = [u.replace("&amp;", "&") for u in re.findall(r"<loc>\s*(?:<!\[CDATA\[)?\s*(.*?)\s*(?:\]\]>)?\s*</loc>", body.decode("utf-8", "replace"), re.S)]
        if b"<sitemapindex" in body:
            queue = sorted(locs, key=lambda u: 0 if "pages" in u.lower() else 1)[:1] + queue
            continue
        pages = [u for u in locs if "/products/" not in u and "/collections/" not in u and allowed(ra, u)][:PER_SITE]
    targets += [{"site": origin, "url": u} for u in pages]

def capture(pass_no):
    for t in targets:
        st, body, ctype = get(t["url"])
        t[f"pass{pass_no}"] = {"status": st, "raw_sha256": hashlib.sha256(body).hexdigest(), "lines": lines(body) if st == 200 and "html" in ctype else None}

capture(1)
time.sleep(gap)
capture(2)
rows = []
for t in targets:
    a, b = t["pass1"], t["pass2"]
    if a["lines"] is None or b["lines"] is None:
        rows.append({"site": t["site"], "url": t["url"], "state": "NOT_HTML_200_IN_BOTH"}); continue
    diff = [d for d in difflib.unified_diff(a["lines"], b["lines"], lineterm="", n=0) if d[:1] in "+-" and not d.startswith(("+++", "---"))]
    rows.append({"site": t["site"], "url": t["url"], "lines": len(a["lines"]), "raw_changed": a["raw_sha256"] != b["raw_sha256"],
                 "text_changed": a["lines"] != b["lines"], "changed_lines": len(diff), "diff_sample": diff[:6]})
print(json.dumps({"gap_seconds": gap, "results": rows, "requests": log}, indent=1, ensure_ascii=False))
