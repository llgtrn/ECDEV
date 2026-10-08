#!/usr/bin/env python3
"""Probe robots.txt edge cases against Scrapy's ProtegoRobotParser, freezing its answers.

Usage: <python with the pinned scrapy and protego installed> scrapy_robots_probe.py > fixture.json
Handcrafted robots.txt texts (grammar quirks, group merging, rule precedence, path matching,
encoding) crossed with URL paths and agents. Unlike the reppy oracle, whose cases are the subset
ECDEV was built to match, these are chosen to find where parsers differ. BENCHMARK_MEASUREMENT:
crafted cases, no live traffic; the answers are the donor's, not a standard's.
"""
import importlib.metadata, itertools, json

import scrapy
from scrapy.robotstxt import ProtegoRobotParser

ROBOTS = {
    "group_merge_same_agent": "User-agent: ecdev\nDisallow: /a\n\nUser-agent: ecdev\nDisallow: /b\n",
    "consecutive_agents_share_rules": "User-agent: other\nUser-agent: ecdev\nDisallow: /a\n",
    "blank_line_between_agent_and_rules": "User-agent: ecdev\n\nDisallow: /a\n",
    "rules_before_any_agent": "Disallow: /a\nUser-agent: *\nDisallow: /b\n",
    "wildcard_group_ignored_when_specific_exists": "User-agent: *\nDisallow: /a\n\nUser-agent: ecdev\nDisallow: /b\n",
    "agent_product_token_match": "User-agent: ecdev/0.1\nDisallow: /a\n\nUser-agent: *\nDisallow: /b\n",
    "agent_substring_of_ua": "User-agent: ecd\nDisallow: /a\n\nUser-agent: *\nDisallow: /b\n",
    "agent_case_insensitive": "User-agent: ECDEV\nDisallow: /a\n",
    "longest_match_wins": "User-agent: *\nDisallow: /a\nAllow: /a/b\n",
    "allow_beats_disallow_on_tie": "User-agent: *\nDisallow: /a\nAllow: /a\n",
    "allow_order_irrelevant": "User-agent: *\nAllow: /a/b\nDisallow: /a\n",
    "wildcard_star_in_path": "User-agent: *\nDisallow: /*.pdf\n",
    "dollar_end_anchor": "User-agent: *\nDisallow: /a$\n",
    "dollar_in_middle": "User-agent: *\nDisallow: /a$b\n",
    "empty_disallow_allows_all": "User-agent: *\nDisallow:\n",
    "empty_allow": "User-agent: *\nDisallow: /a\nAllow:\n",
    "disallow_root": "User-agent: *\nDisallow: /\n",
    "allow_root_then_disallow_all": "User-agent: *\nAllow: /\nDisallow: /\n",
    "query_in_rule": "User-agent: *\nDisallow: /search?q=\n",
    "percent_encoded_rule": "User-agent: *\nDisallow: /caf%C3%A9\n",
    "raw_utf8_rule": "User-agent: *\nDisallow: /café\n",
    "percent_case_in_rule": "User-agent: *\nDisallow: /a%2fb\n",
    "unreserved_escaped_in_rule": "User-agent: *\nDisallow: /%61bc\n",
    "directive_case": "USER-AGENT: *\nDISALLOW: /a\n",
    "directive_without_colon": "User-agent *\nDisallow /a\n",
    "space_before_colon": "User-agent : *\nDisallow : /a\n",
    "inline_comment": "User-agent: *\nDisallow: /a # not /b\n",
    "tab_separated": "User-agent:\t*\nDisallow:\t/a\n",
    "crlf": "User-agent: *\r\nDisallow: /a\r\n",
    "cr_only": "User-agent: *\rDisallow: /a\r",
    "bom_prefix": "﻿User-agent: *\nDisallow: /a\n",
    "no_trailing_newline": "User-agent: *\nDisallow: /a",
    "leading_whitespace": "  User-agent: *\n  Disallow: /a\n",
    "unknown_directive": "User-agent: *\nNoindex: /a\nDisallow: /b\n",
    "sitemap_between": "User-agent: *\nSitemap: https://x.example/s.xml\nDisallow: /a\n",
    "crawl_delay_int": "User-agent: *\nCrawl-delay: 5\n",
    "crawl_delay_float": "User-agent: *\nCrawl-delay: 1.5\n",
    "crawl_delay_invalid": "User-agent: *\nCrawl-delay: soon\n",
    "crawl_delay_negative": "User-agent: *\nCrawl-delay: -3\n",
    "crawl_delay_specific_over_wildcard": "User-agent: *\nCrawl-delay: 9\n\nUser-agent: ecdev\nCrawl-delay: 2\n",
    "request_rate": "User-agent: *\nRequest-rate: 1/10\nDisallow: /a\n",
    "disallow_without_slash": "User-agent: *\nDisallow: a\n",
    "trailing_slash_dir": "User-agent: *\nDisallow: /dir/\n",
    "empty_file": "",
    "only_comments": "# nothing here\n",
    "html_error_page": "<html><body>503 Service Unavailable</body></html>",
    "very_long_rule": "User-agent: *\nDisallow: /" + "a" * 2000 + "\n",
}
PATHS = ["/", "/a", "/a/b", "/b", "/a$b", "/abc", "/x.pdf", "/x.pdf?y=1", "/dir", "/dir/", "/dir/x", "/search?q=1", "/search?q=", "/caf%C3%A9", "/café", "/a%2Fb", "/a/b", "/A"]
AGENTS = ["ECDEV", "ecdev/0.1", "Googlebot"]
cases = []
for (name, text), path, agent in itertools.product(ROBOTS.items(), PATHS, AGENTS):
    url = "https://shop.example" + path
    try:
        p = ProtegoRobotParser(text.encode("utf-8"), None)
        r = {"allowed": bool(p.allowed(url, agent)), "crawl_delay_seconds": p.crawl_delay(agent)}
    except Exception as e:  # noqa: BLE001
        r = {"error": type(e).__name__}
    cases.append({"name": name, "path": path, "agent": agent, **r})
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "scrapy_version": scrapy.__version__,
                  "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "protego_version": importlib.metadata.version("protego"),
                  "texts": ROBOTS, "cases": cases}, ensure_ascii=False))
