#!/usr/bin/env python3
"""Freeze Scrapy's robots decisions (ProtegoRobotParser) on the cases of the reppy robots oracle.

Usage: <python with the pinned scrapy and protego installed> scrapy_robots_oracle.py <reppy-robots.json> > fixture.json
Each case (robots.txt text, URL, agent) is parsed by scrapy.robotstxt.ProtegoRobotParser, Scrapy's
default parser (ROBOTSTXT_PARSER), and asked allowed(url, agent) and crawl_delay(agent). The fixture
keeps the answers in the same case order. BENCHMARK_MEASUREMENT: crafted cases, no live traffic.
"""
import json, sys

import importlib.metadata, scrapy
from scrapy.robotstxt import ProtegoRobotParser

src = json.load(open(sys.argv[1]))
out = []
for c in src["cases"]:
    try:
        p = ProtegoRobotParser(c["robots"].encode("utf-8"), None)
        out.append({"allowed": bool(p.allowed(c["url"], c["agent"])), "crawl_delay_seconds": p.crawl_delay(c["agent"])})
    except Exception as e:  # noqa: BLE001
        out.append({"error": type(e).__name__})
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "scrapy_version": scrapy.__version__,
                  "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "protego_version": importlib.metadata.version("protego"),
                  "cases_from": "reppy-robots.json (same order)", "results": out}, ensure_ascii=False))
