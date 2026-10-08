#!/usr/bin/env python3
"""Freeze which HTTP statuses Scrapy's RetryMiddleware retries (default settings), 100 to 599.

Usage: <python with the pinned scrapy installed> scrapy_retry_oracle.py > fixture.json
Each status is run through RetryMiddleware.process_response for a GET request. The fixture keeps
whether a retry request came back, the priority change and whether the retry bypasses the
duplicate filter. BENCHMARK_MEASUREMENT: crafted responses, no live traffic.
"""
import json

import scrapy
from scrapy import Request
from scrapy.downloadermiddlewares.retry import RetryMiddleware
from scrapy.http import Response
from scrapy.utils.test import get_crawler

crawler = get_crawler()
spider = crawler._create_spider("probe") if hasattr(crawler, "_create_spider") else None
crawler.spider = spider
mw = RetryMiddleware.from_crawler(crawler)
results = {}
for status in range(100, 600):
    req = Request("https://shop.example/p")
    r = mw.process_response(req, Response(req.url, status=status))
    results[status] = {"retried": isinstance(r, Request)}
    if isinstance(r, Request):
        results[status].update(priority_delta=r.priority - req.priority, dont_filter=r.dont_filter, retry_times=r.meta.get("retry_times"))
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "scrapy_version": scrapy.__version__,
                  "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "settings": {"RETRY_TIMES": 2, "RETRY_HTTP_CODES": [500, 502, 503, 504, 522, 524, 408, 429], "RETRY_PRIORITY_ADJUST": -1},
                  "statuses": results}))
