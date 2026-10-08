#!/usr/bin/env python3
"""Freeze Scrapy's request identity on rewritten real URLs, to measure ECDEV against it.

Usage: <python with the pinned scrapy installed> scrapy_url_identity_oracle.py <urls.json> > fixture.json
urls.json is a list of public URLs from live captures (links in posts, kept outside the repository);
profile and social-network URLs are dropped. Each URL is rewritten in ways that change how a URL is
written but, by the URL standards, not what it names (query order, host case, default port,
fragment, percent-escape case, dot segments, an extra tracking parameter, empty path, scheme case,
an escaped unreserved letter, repeated slashes, a bare trailing '?') and in ways that name something else (path case,
parameter value, parameter name). For each pair the fixture records whether scrapy.utils.request.
fingerprint (method GET, no body, fragments dropped) treats the two as one request. The Rust test
domain/commerce/tests/scrapy_url_identity.rs computes ECDEV's answer for the same pairs and pins
the comparison. BENCHMARK_MEASUREMENT on captures kept outside the repository; selection is
deterministic (sorted by SHA-256 of the URL, at most PER_KIND per kind).
"""
import hashlib, json, re, sys
from urllib.parse import urlsplit, urlunsplit

import scrapy, w3lib
from scrapy import Request
from scrapy.utils.request import fingerprint

PER_KIND = 40
SOCIAL = re.compile(r"(bsky\.|mastodon|/@|/profile/|x\.com/|twitter\.com/|t\.co/)", re.I)


def fp(u):
    return fingerprint(Request(u)).hex()


def split(u):
    return urlsplit(u)


def q_reverse(u):
    s = split(u)
    parts = s.query.split("&")
    return urlunsplit(s._replace(query="&".join(reversed(parts)))) if len(parts) > 1 else None


def host_upper(u):
    s = split(u)
    return urlunsplit(s._replace(netloc=s.netloc.upper())) if s.hostname and "@" not in s.netloc else None


def default_port(u):
    s = split(u)
    if s.port is not None or "@" in s.netloc or not s.hostname:
        return None
    return urlunsplit(s._replace(netloc=s.netloc + (":443" if s.scheme == "https" else ":80")))


def fragment_add(u):
    return u + "#reviews" if "#" not in u else None


def pct_hex_case(u):
    v = re.sub(r"%([0-9A-F]{2})", lambda m: "%" + m.group(1).lower(), u)
    return v if v != u else None


def dot_segment(u):
    s = split(u)
    if not s.path.startswith("/") or s.path == "/":
        return None
    return urlunsplit(s._replace(path="/./" + s.path.lstrip("/")))


def tracking_add(u):
    s = split(u)
    return urlunsplit(s._replace(query=(s.query + "&" if s.query else "") + "utm_source=x"))


def empty_path(u):
    s = split(u)
    return urlunsplit(s._replace(path="")) if s.path == "/" and not s.query else None


def scheme_upper(u):
    return "HTTP" + u[4:] if u.startswith("http://") else ("HTTPS" + u[5:] if u.startswith("https://") else None)


def escaped_unreserved(u):
    s = split(u)
    m = re.search(r"[a-z]", s.path)
    if not m:
        return None
    p = s.path[: m.start()] + "%{:02X}".format(ord(m.group(0))) + s.path[m.end():]
    return urlunsplit(s._replace(path=p))


def repeated_slash(u):
    s = split(u)
    return urlunsplit(s._replace(path=s.path.replace("/", "//", 1))) if s.path.count("/") >= 2 else None


def empty_query_marker(u):
    return u + "?" if "?" not in u and "#" not in u else None


def path_case(u):
    s = split(u)
    return urlunsplit(s._replace(path=s.path.swapcase())) if re.search(r"[A-Za-z]", s.path) else None


def value_change(u):
    s = split(u)
    return urlunsplit(s._replace(query=s.query + "0")) if re.search(r"=[^&]+$", s.query) else None


def name_change(u):
    s = split(u)
    return urlunsplit(s._replace(query="x" + s.query)) if "=" in s.query else None


SAME = [("query_order", q_reverse), ("host_case", host_upper), ("default_port", default_port),
        ("fragment_added", fragment_add), ("percent_hex_case", pct_hex_case), ("dot_segment", dot_segment),
        ("tracking_parameter_added", tracking_add), ("empty_path", empty_path), ("scheme_case", scheme_upper),
        ("escaped_unreserved_letter", escaped_unreserved), ("repeated_slash", repeated_slash), ("empty_query_marker", empty_query_marker)]
DIFFERENT = [("path_case", path_case), ("parameter_value_changed", value_change), ("parameter_name_changed", name_change)]

urls = [u for u in json.load(open(sys.argv[1])) if not SOCIAL.search(u) and len(u) < 400]
urls.sort(key=lambda u: hashlib.sha256(u.encode()).hexdigest())
cases = []
for kind, fn in SAME + DIFFERENT:
    n = 0
    for u in urls:
        v = fn(u)
        if v is None or v == u:
            continue
        cases.append({"kind": kind, "intent": "SAME_RESOURCE" if (kind, fn) in SAME else "DIFFERENT_RESOURCE", "a": u, "b": v, "scrapy_same": fp(u) == fp(v)})
        n += 1
        if n == PER_KIND:
            break
out = {"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "oracle": "scrapy.utils.request.fingerprint(Request(url)), method GET, no body, fragments dropped",
       "scrapy_version": scrapy.__version__, "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "w3lib_version": w3lib.__version__,
       "source_urls": len(urls), "per_kind_limit": PER_KIND, "cases": cases}
json.dump(out, sys.stdout, indent=0, ensure_ascii=False)
