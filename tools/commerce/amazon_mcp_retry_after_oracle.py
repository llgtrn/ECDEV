#!/usr/bin/env python3
"""Freeze how the coaxon/amazon-mcp 429 handler reads a Retry-After header.

Runs the donor's own raise_on_429 on a 429 response for each header value and
records the wait it would honour, or the exception it raises. Usage:
  python3 amazon_mcp_retry_after_oracle.py <donor checkout> <out.json>
The donor needs httpx importable. The commit is recorded from the checkout.
"""
import json, subprocess, sys
donor, out = sys.argv[1], sys.argv[2]
sys.path.insert(0, donor)
import httpx
from amazon_mcp.clients.http_retry import raise_on_429
from amazon_mcp.clients.rate_limit import RateLimitError

VALUES = ["0", "1", "7", "120", "86400", "1.5", " 7 ", "+7", "-1", "1e3", "inf", "nan", "", "abc",
          "7s", "Wed, 21 Oct 2015 07:28:00 GMT", "Wednesday, 21-Oct-15 07:28:00 GMT",
          "Wed Oct 21 07:28:00 2015", "99999999999999999999", "0x10"]
cases = []
for v in VALUES:
    resp = httpx.Response(429, headers={"Retry-After": v}, request=httpx.Request("GET", "https://x"))
    try:
        raise_on_429(resp)
        r = {"outcome": "NO_ERROR"}
    except RateLimitError as e:
        r = {"outcome": "WAIT", "seconds": repr(e.retry_after)}
    except Exception as e:  # the donor lets a parse failure escape
        r = {"outcome": "EXCEPTION", "type": type(e).__name__}
    cases.append({"header": v, **r})
sha = subprocess.check_output(["git", "-C", donor, "rev-parse", "HEAD"], text=True).strip()
json.dump({"donor": "coaxon/amazon-mcp", "commit": sha, "httpx": httpx.__version__,
           "function": "amazon_mcp.clients.http_retry.raise_on_429", "cases": cases},
          open(out, "w"), indent=1, ensure_ascii=False)
for c in cases: print(c)
