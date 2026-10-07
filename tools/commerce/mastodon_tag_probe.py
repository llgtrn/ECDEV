#!/usr/bin/env python3
"""Bounded probe of the public Mastodon tag record that ECDEV's MASTODON_TAG source reads.

Usage: mastodon_tag_probe.py > result.json
An API-contract probe through the session's egress proxy, NOT an ECDEV runtime capture: nothing
it reads is stored as evidence or labelled LIVE. It checks the assumptions the native parser
makes (robots allows the path; history is at most 31 UTC-midnight days with string counts and
accounts <= uses; the newest day is the open one; the name echoes the requested tag; and what an
unknown tag returns), and the two
trending lists ecdev.trend.feeds reads (Mastodon trends/tags, Bluesky unspecced getTrends). ECDEV user agent, 1.5 s between requests; bodies are hashed, never kept.
"""
import hashlib, json, time, urllib.request

UA = "ECDEV"
last = [0.0]
log = []

def get(url):
    wait = 1.5 - (time.time() - last[0])
    if wait > 0:
        time.sleep(wait)
    last[0] = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": UA}), timeout=25) as r:
            body, status, headers = r.read(), r.status, {k.lower(): v for k, v in r.headers.items()}
    except urllib.error.HTTPError as e:
        body, status, headers = e.read(), e.code, {k.lower(): v for k, v in e.headers.items()}
    log.append({"url": url, "status": status, "bytes": len(body), "sha256": hashlib.sha256(body).hexdigest(),
                "content_type": headers.get("content-type"), "ratelimit_limit": headers.get("x-ratelimit-limit")})
    return status, body

def robots_allows(text, path):
    # Longest-match over the '*' group, enough for the paths probed here.
    group, rules = False, []
    for line in text.splitlines():
        k, _, v = line.partition(":")
        k, v = k.strip().lower(), v.split("#")[0].strip()
        if k == "user-agent":
            group = v == "*"
        elif group and k in ("allow", "disallow") and v:
            rules.append((len(v), k == "allow", v))
    hits = [r for r in rules if path.startswith(r[2])]
    return max(hits)[1] if hits else True

out = {"probe_class": "API_CONTRACT_PROBE_NOT_ECDEV_RUNTIME_CAPTURE", "probed_at": int(time.time()), "instances": {}}
now = int(time.time())
for instance, tags in (("mastodon.social", ["matcha", "matchalatte"]), ("fosstodon.org", ["rust"])):
    rs, rb = get(f"https://{instance}/robots.txt")
    entry = {"robots_status": rs, "robots_allows_tags_api": robots_allows(rb.decode("utf-8", "replace"), "/api/v1/tags/x") if rs == 200 else None, "tags": {}}
    for tag in tags:
        s, b = get(f"https://{instance}/api/v1/tags/{tag}")
        d = json.loads(b) if s == 200 else None
        if d is None:
            entry["tags"][tag] = {"status": s}
            continue
        h = d.get("history", [])
        days = [int(x["day"]) for x in h]
        entry["tags"][tag] = {
            "status": s, "name_echo_lowercase_matches": d.get("name", "").lower() == tag,
            "history_len": len(h), "counts_are_strings": all(isinstance(x["uses"], str) and isinstance(x["accounts"], str) for x in h),
            "days_midnight_utc": all(x % 86400 == 0 for x in days), "newest_first": days == sorted(days, reverse=True),
            "newest_day_is_open": bool(days) and days[0] <= now < days[0] + 86400,
            "accounts_le_uses": all(int(x["accounts"]) <= int(x["uses"]) for x in h),
            "uses_series_oldest_first": [int(x["uses"]) for x in reversed(h)], "accounts_series_oldest_first": [int(x["accounts"]) for x in reversed(h)],
        }
    out["instances"][instance] = entry
s, b = get("https://mastodon.social/api/v1/tags/ecdevnonexistenttag0000")
u = json.loads(b) if s == 200 else {}
out["unknown_tag"] = {"status": s, "history_len": len(u.get("history", [])), "all_zero": all(x["uses"] == "0" for x in u.get("history", [])),
                      "meaning": "AN_UNUSED_TAG_AND_AN_UNKNOWN_TAG_ARE_INDISTINGUISHABLE_ZEROS_ARE_INSTANCE_OBSERVED_ZERO_NOT_ABSENCE_ELSEWHERE"}
# The tag timeline read by the MASTODON post source: page size, id-encoded receipt time, id-range
# slices and max_id paging. Only counts and booleans are kept, never posts.
import datetime
def created(x):
    return int(datetime.datetime.fromisoformat(x["created_at"].replace("Z", "+00:00")).timestamp())
s, b = get("https://mastodon.social/api/v1/timelines/tag/coffee?limit=40")
p1 = json.loads(b) if s == 200 else []
tl = {"status": s, "page_size": len(p1)}
if p1:
    skew = [abs(((int(x["id"]) >> 16) // 1000) - created(x)) for x in p1]
    tl["id_time_minus_created_at_max_abs_seconds"] = max(skew)
    tl["has_global_uri"] = all(x.get("uri") for x in p1)
    until = created(p1[0]) - 3 * 3600
    since = until - 3600
    s2, b2 = get(f"https://mastodon.social/api/v1/timelines/tag/coffee?limit=40&max_id={((until + 1) * 1000) << 16}&since_id={((since + 1) * 1000) << 16}")
    sl = json.loads(b2) if s2 == 200 else []
    tl["slice"] = {"status": s2, "returned": len(sl), "all_received_within": all(since < (int(x["id"]) >> 16) // 1000 <= until for x in sl)}
    s3, b3 = get(f"https://mastodon.social/api/v1/timelines/tag/coffee?limit=40&max_id={p1[-1]['id']}")
    p2 = json.loads(b3) if s3 == 200 else []
    tl["next_page"] = {"status": s3, "returned": len(p2), "overlap_with_first": len({x["id"] for x in p1} & {x["id"] for x in p2}),
                       "strictly_older_ids": all(int(x["id"]) < int(p1[-1]["id"]) for x in p2)}
    root = max(p1, key=lambda x: x.get("replies_count") or 0)
    s4, b4 = get(f"https://mastodon.social/api/v1/statuses/{root['id']}/context")
    c = json.loads(b4) if s4 == 200 else {}
    ids = {root["id"]} | {x["id"] for x in c.get("descendants", [])}
    tl["context"] = {"status": s4, "root_replies_count": root.get("replies_count"), "descendants": len(c.get("descendants", [])),
                     "every_parent_in_response": all(x.get("in_reply_to_id") in ids for x in c.get("descendants", [])),
                     "unauthenticated_limits_from_source": "descendants 60, depth 20 (mastodon app/controllers/api/v1/statuses/contexts_controller.rb, main, read 2026-10-07)"}
out["mastodon_tag_timeline"] = tl

# Instance-wide trending lists read by ecdev.trend.feeds.
s, b = get("https://mastodon.social/api/v1/trends/tags?limit=10")
t = json.loads(b) if s == 200 else []
out["mastodon_trends"] = {"status": s, "entries": len(t), "all_have_name_and_7_day_history": all("name" in x and len(x.get("history", [])) == 7 for x in t),
                          "names": [x.get("name") for x in t]}
s, b = get("https://public.api.bsky.app/robots.txt")
out["bluesky_public_appview_robots"] = {"status": s, "allows_xrpc": robots_allows(b.decode("utf-8", "replace"), "/xrpc/app.bsky.unspecced.getTrends") if s == 200 else None}
s, b = get("https://public.api.bsky.app/xrpc/app.bsky.unspecced.getTrends?limit=10")
t = (json.loads(b) if s == 200 else {}).get("trends", [])
out["bluesky_trends"] = {"status": s, "entries": len(t), "fields": sorted({k for x in t for k in x}),
                         "entries_carry_actor_handles": any(x.get("actors") for x in t), "statuses": sorted({x.get("status") for x in t if x.get("status")}),
                         "labels": [x.get("displayName") for x in t], "api_namespace": "app.bsky.unspecced (no stability guarantee)"}
out["requests"] = log
print(json.dumps(out, indent=1))
