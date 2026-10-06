#!/usr/bin/env python3
"""Bounded benchmark: which structured-data formats carry product facts on real product pages.

Usage: structured_data_benchmark.py <home URL>... > result.json
Robots-respecting (404 = allow all), ECDEV user agent, 1.5 s between requests, product URLs
from the first uncompressed product sitemap the site's robots.txt leads to, at most 5 pages per
site, 4 MiB per body. extruct (from the donor checkout) is the measuring instrument only; bodies
are hashed, never stored.
"""
import hashlib, json, re, sys, time, urllib.parse, urllib.request, urllib.robotparser
import extruct

UA = "ECDEV"
PRODUCT = re.compile(r"/(products?|p|dp|item|items|ip)/[^/?#]+", re.I)
MAX_BYTES = 4 * 1024 * 1024
PAGES = 5
last = [0.0]
log = []

def get(url):
    wait = 1.5 - (time.time() - last[0])
    if wait > 0:
        time.sleep(wait)
    last[0] = time.time()
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            body, status, ctype = r.read(MAX_BYTES + 1), r.status, r.headers.get("Content-Type", "")
    except urllib.error.HTTPError as e:
        body, status, ctype = b"", e.code, ""
    except Exception as e:
        body, status, ctype = b"", None, ""
        log.append({"url": url, "error": type(e).__name__})
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest()})
    return status, body, ctype

def locs(xml):
    t = xml.decode("utf-8", "replace")
    kind = "index" if "<sitemapindex" in t else "urlset" if "<urlset" in t else None
    return kind, [m.strip().replace("&amp;", "&") for m in re.findall(r"<loc>\s*(?:<!\[CDATA\[)?\s*(.*?)\s*(?:\]\]>)?\s*</loc>", t, re.S)]

def has_product_type(t):
    t = t if isinstance(t, list) else [t]
    return any(isinstance(x, str) and re.search(r"(^|[/#:])Product$|IndividualProduct|ProductModel|ProductGroup", x) for x in t)

def jsonld_products(items):
    found = []
    def walk(v):
        if isinstance(v, dict):
            if has_product_type(v.get("@type")):
                found.append(v)
            for x in v.values():
                walk(x)
        elif isinstance(v, list):
            for x in v:
                walk(x)
    walk(items)
    return found

def offer_price(p):
    o = p.get("offers") or p.get("properties", {}).get("offers")
    return o is not None

def analyse(html, url):
    data = extruct.extract(html, base_url=url, syntaxes=["json-ld", "microdata", "rdfa", "microformat", "opengraph"], uniform=False, errors="ignore")
    jl = jsonld_products(data.get("json-ld", []))
    md = [i for i in data.get("microdata", []) if has_product_type(i.get("type"))]
    rdfa_types = []
    rdfa_price = False
    for node in data.get("rdfa", []):
        types = node.get("@type", [])
        if has_product_type(types):
            rdfa_types.append(types)
        if any(re.search(r"schema\.org/(price|offers)$|purl\.org/goodrelations/v1#hasCurrencyValue", k) for k in node):
            rdfa_price = True
    mf = [i for i in data.get("microformat", []) if "h-product" in i.get("type", [])]
    og = data.get("opengraph", [])
    og_product = any(any(k in ("product:price:amount", "og:price:amount") for k, _ in (o.get("properties") or [])) or "product" in str(dict(o.get("properties") or []).get("og:type", "")) for o in og)
    native = bool(jl) or bool(md) or og_product
    return {
        "jsonld_product": bool(jl), "jsonld_offer": any(offer_price(p) for p in jl),
        "microdata_product": bool(md),
        "opengraph_product": og_product,
        "rdfa_product": bool(rdfa_types) or rdfa_price,
        "microformat_h_product": bool(mf),
        "read_by_ecdev": native,
        "only_rdfa_or_microformat": (not native) and (bool(rdfa_types) or rdfa_price or bool(mf)),
    }

results = []
for home in sys.argv[1:]:
    origin = urllib.parse.urlsplit(home)._replace(path="", query="", fragment="").geturl()
    s, robots, _ = get(origin + "/robots.txt")
    rp = urllib.robotparser.RobotFileParser()
    text = robots.decode("utf-8", "replace")
    rp.parse(text.splitlines() if s == 200 else [])
    if s == 404:
        rp.allow_all = True
    row = {"site": origin, "robots_status": s}
    if s not in (200, 404):
        row["state"] = "ROBOTS_UNKNOWN"; results.append(row); continue
    queue = [l.split(":", 1)[1].strip() for l in text.splitlines() if l.lower().startswith("sitemap:")] if s == 200 else []
    products, fetched = [], 0
    while queue and not products and fetched < 4:
        sm = queue.pop(0)
        if sm.endswith(".gz") or not rp.can_fetch(UA, sm):
            continue
        st, body, _ = get(sm); fetched += 1
        if st != 200:
            continue
        kind, ls = locs(body)
        if kind == "index":
            queue = sorted(ls, key=lambda u: 0 if "product" in u.lower() else 1) + queue
        elif kind == "urlset":
            products = [u for u in ls if PRODUCT.search(urllib.parse.urlsplit(u).path) and rp.can_fetch(UA, u)][:PAGES]
    row["sitemaps_fetched"] = fetched
    if not products:
        row["state"] = "NO_PRODUCT_URLS_FROM_SITEMAPS"; results.append(row); continue
    pages = []
    for u in products:
        st, body, ctype = get(u)
        if st != 200 or "html" not in ctype:
            pages.append({"url": u, "status": st, "state": "NOT_HTML_200"}); continue
        try:
            pages.append({"url": u, "status": st, **analyse(body.decode("utf-8", "replace"), u)})
        except Exception as e:
            pages.append({"url": u, "status": st, "state": "EXTRACT_ERROR", "error": type(e).__name__})
    row.update(state="SAMPLED", pages=pages)
    results.append(row)
print(json.dumps({"extruct_commit": sys.argv[0] and "dc3bf7d2209ecf421222afc90d3d2dd8c6fd23cf", "results": results, "requests": log}, indent=1))
