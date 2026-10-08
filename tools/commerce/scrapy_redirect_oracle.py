#!/usr/bin/env python3
"""Freeze Scrapy's redirect decisions on the cases of the httpx redirect oracle.

Usage: <python with the pinned scrapy installed> scrapy_redirect_oracle.py <httpx-redirect-oracle.json> > fixture.json
Each case (base URL, status, Location header or none) is run through scrapy.downloadermiddlewares.
redirect.RedirectMiddleware.process_response on a GET request for the base URL with a response of
that status. The fixture keeps, per case and in the same order, whether Scrapy follows and to
which URL (fragment included, as Scrapy re-attaches the request's own). Cases the middleware
raises on are recorded as errors. BENCHMARK_MEASUREMENT: crafted cases, no live traffic.
"""
import json, sys

import scrapy
from scrapy import Request
from scrapy.downloadermiddlewares.redirect import RedirectMiddleware
from scrapy.http import Response
from scrapy.utils.test import get_crawler

source = json.load(open(sys.argv[1]))
crawler = get_crawler()
mw = RedirectMiddleware.from_crawler(crawler)
out = []
for c in source["cases"]:
    headers = {} if c["location"] is None else {"Location": c["location"]}
    try:
        req = Request(c["base"])
        res = Response(c["base"], status=c["status"], headers=headers)
        r = mw.process_response(req, res)
        if isinstance(r, Request):
            out.append({"follows": True, "target": r.url})
        else:
            out.append({"follows": False})
    except Exception as e:  # noqa: BLE001
        out.append({"follows": None, "error": type(e).__name__})
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "scrapy_version": scrapy.__version__,
                  "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "cases_from": "httpx-redirect-oracle.json (same order)",
                  "results": out}, ensure_ascii=False))
