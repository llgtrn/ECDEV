#!/usr/bin/env python3
"""Freeze Scrapy's HTTP cache freshness (RFC2616Policy) on a grid of response header sets.

Usage: <python with the pinned scrapy installed> scrapy_cache_policy_oracle.py > fixture.json
For each combination of Cache-Control, Expires, Last-Modified and Age (Date always present, the
response just received so now == Date), the fixture records what Scrapy's policy computes: the
freshness lifetime and the remaining freshness (lifetime minus current age, floor 0), and whether
a stored copy would be stored at all. BENCHMARK_MEASUREMENT: crafted headers, no live traffic.
"""
import itertools, json, sys
from email.utils import formatdate

import scrapy
from scrapy import Request
from scrapy.extensions.httpcache import RFC2616Policy
from scrapy.http import Response
from scrapy.settings import Settings

NOW = 1_800_000_000
CC = ["", "max-age=60", "max-age=0", "max-age=abc", "no-cache", "no-store", "max-age=60, no-cache", "public, max-age=31536000", "s-maxage=30", "private, max-age=120", "max-age=90000"]
EXPIRES = [None, "+120", "-120", "garbage"]
LASTMOD = [None, "-864000", "+3600"]
AGE = [None, "30", "abc", "200"]
policy = RFC2616Policy(Settings())


def http_date(offset):
    return formatdate(NOW + int(offset), usegmt=True)


cases = []
for cc, ex, lm, age in itertools.product(CC, EXPIRES, LASTMOD, AGE):
    h = {"Date": http_date(0)}
    if cc:
        h["Cache-Control"] = cc
    if ex is not None:
        h["Expires"] = "garbage" if ex == "garbage" else http_date(ex)
    if lm is not None:
        h["Last-Modified"] = http_date(lm)
    if age is not None:
        h["Age"] = age
    r = Response("https://shop.example/p", status=200, headers=h)
    req = Request("https://shop.example/p")
    stored = policy.should_cache_response(r, req)
    lifetime = policy._compute_freshness_lifetime(r, req, NOW)
    current = policy._compute_current_age(r, req, NOW)
    nocache = b"no-cache" in policy._parse_cachecontrol(r)
    cases.append({"headers": dict(h), "cache_control": cc, "expires_offset": ex, "last_modified_offset": lm, "age": age,
                  "scrapy_stored": stored, "scrapy_no_cache": nocache, "scrapy_lifetime": lifetime, "scrapy_current_age": current,
                  "scrapy_remaining": 0 if nocache else max(0, lifetime - current)})
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "scrapy_version": scrapy.__version__,
                  "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "now": NOW, "policy": "RFC2616Policy defaults", "cases": cases}))
