#!/usr/bin/env python3
"""Bounded benchmark: product URLs reachable from robots-declared sitemaps versus a home page.

Usage: sitemap_discovery_benchmark.py <home URL>... > result.json
Robots-respecting (404 = allow all), ECDEV user agent, 1 s between requests, at most 6 sitemaps
per site, 10 MiB per body. Bodies are hashed, never stored.
"""
import gzip, hashlib, json, re, sys, time, urllib.parse, urllib.request, urllib.robotparser
UA = "ECDEV"
SITES = sys.argv[1:]
PRODUCT = re.compile(r"/(products?|p|dp|item|items)/[^/?#]+", re.I)
MAX_BYTES = 10 * 1024 * 1024
MAX_SITEMAPS = 6
log = []
last = [0.0]
def get(url):
    wait = 1.0 - (time.time() - last[0])
    if wait > 0: time.sleep(wait)
    last[0] = time.time()
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept-Encoding": "gzip"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            body = r.read(MAX_BYTES + 1)
            status = r.status
            if r.headers.get("Content-Encoding") == "gzip" or url.endswith(".gz"):
                try: body = gzip.decompress(body)
                except Exception: pass
    except urllib.error.HTTPError as e:
        status, body = e.code, b""
    except Exception as e:
        status, body = None, b""
        log.append({"url": url, "error": type(e).__name__})
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    return status, body
def locs(xml):
    text = xml.decode("utf-8", "replace")
    kind = "sitemapindex" if "<sitemapindex" in text else ("urlset" if "<urlset" in text else "unknown")
    out = [re.sub(r"^<!\[CDATA\[|\]\]>$", "", m.strip()).replace("&amp;", "&") for m in re.findall(r"<loc>\s*(.*?)\s*</loc>", text, re.S)]
    return kind, out
results = []
for home in SITES:
    origin = urllib.parse.urlsplit(home)._replace(path="", query="", fragment="").geturl()
    s, robots = get(origin + "/robots.txt")
    rp = urllib.robotparser.RobotFileParser(); rp.parse(robots.decode("utf-8", "replace").splitlines() if s == 200 else [])
    if s == 404: rp.allow_all = True
    declared = list(dict.fromkeys(l.split(":", 1)[1].strip() for l in robots.decode("utf-8", "replace").splitlines() if l.lower().startswith("sitemap:"))) if s == 200 else []
    row = {"site": origin, "robots_status": s, "declared_sitemaps": declared}
    if s not in (200, 404) or not rp.can_fetch(UA, home):
        row["home"] = "ROBOTS_DENIED_OR_UNKNOWN"; results.append(row); continue
    hs, html = get(home)
    links = {urllib.parse.urljoin(home, h).split("#")[0] for h in re.findall(r'href="([^"]+)"', html.decode("utf-8", "replace"))}
    same = {l for l in links if urllib.parse.urlsplit(l).netloc == urllib.parse.urlsplit(home).netloc}
    home_products = {urllib.parse.urlsplit(l).path.rstrip("/") for l in same if PRODUCT.search(urllib.parse.urlsplit(l).path)}
    row.update(home_status=hs, home_links=len(same), home_product_urls=len(home_products))
    queue, seen, sm_products, fetched, truncated = list(declared), set(), set(), 0, False
    while queue:
        sm = queue.pop(0)
        if sm in seen: continue
        seen.add(sm)
        if fetched >= MAX_SITEMAPS: truncated = True; break
        if not rp.can_fetch(UA, sm): continue
        st, body = get(sm); fetched += 1
        if st != 200: continue
        kind, ls = locs(body)
        if kind == "sitemapindex":
            # Product sitemaps first: the question is product-URL coverage.
            queue = sorted(ls, key=lambda u: 0 if "product" in u.lower() else 1) + queue
        else:
            sm_products |= {urllib.parse.urlsplit(u).path.rstrip("/") for u in ls if PRODUCT.search(urllib.parse.urlsplit(u).path)}
    row.update(sitemaps_fetched=fetched, sitemap_budget_exhausted=truncated, sitemap_product_urls=len(sm_products),
               home_products_in_sitemaps=len(home_products & sm_products), home_products_missing_from_sitemaps=len(home_products - sm_products), missing_examples=sorted(home_products - sm_products)[:5])
    results.append(row)
print(json.dumps({"results": results, "requests": log}, indent=1))
