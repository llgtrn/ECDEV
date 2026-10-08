#!/usr/bin/env python3
"""Freeze gzip streams made by CPython's zlib, with their expected output, to validate ECDEV's gunzip.

Usage: python3 gzip_oracle.py > fixture.json
Cases cover every DEFLATE block type (stored, fixed Huffman, dynamic Huffman) across compression
levels 0-9 and strategies (default, filtered, huffman-only, RLE, fixed), inputs from empty to
200 KB (random bytes to 70 KB) (text, repeated, random, sitemap XML), gzip header variants (name, comment, extra field,
header CRC), concatenated members, and malformed streams (truncated at several points, flipped
bits, wrong CRC, wrong length, bad method). The expectation for a valid case is the output's
SHA-256 and length; for a malformed one, that it must fail. For the malformed ones the fixture
also records what Scrapy's gunzip returns (partial output vs error) when scrapy is importable.
Deterministic: seeded generator, mtime fixed. BENCHMARK_MEASUREMENT: crafted streams.
"""
import gzip, hashlib, json, random, struct, sys, zlib

rnd = random.Random(20261008)


def sample(kind, n):
    if kind == "text":
        words = ["matcha", "whisk", "bamboo", "price", "JPY", "https://shop.example/p/", "<loc>", "</loc>", "item", "2026-10-08"]
        return " ".join(rnd.choice(words) for _ in range(n // 6 + 1)).encode()[:n]
    if kind == "repeat":
        return (b"abcabcabd" * (n // 9 + 1))[:n]
    if kind == "random":
        return bytes(rnd.getrandbits(8) for _ in range(n))
    if kind == "zeros":
        return b"\0" * n
    if kind == "sitemap":
        rows = "".join(f"<url><loc>https://shop.example/p/{i}</loc><lastmod>2026-09-{i % 28 + 1:02d}</lastmod></url>\n" for i in range(n // 90 + 1))
        return (f'<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n{rows}</urlset>\n').encode()[:n] if n < 200 else (f'<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n{rows}</urlset>\n').encode()
    raise ValueError(kind)


def member(data, level=9, strategy=zlib.Z_DEFAULT_STRATEGY, name=None, comment=None, extra=None, hcrc=False):
    flg = (2 if hcrc else 0) | (4 if extra is not None else 0) | (8 if name else 0) | (16 if comment else 0)
    head = struct.pack("<BBBBIBB", 0x1F, 0x8B, 8, flg, 0, 0, 255)
    if extra is not None:
        head += struct.pack("<H", len(extra)) + extra
    if name:
        head += name.encode() + b"\0"
    if comment:
        head += comment.encode() + b"\0"
    if hcrc:
        head += struct.pack("<H", zlib.crc32(head) & 0xFFFF)
    c = zlib.compressobj(level, zlib.DEFLATED, -15, 9, strategy)
    body = c.compress(data) + c.flush()
    return head + body + struct.pack("<II", zlib.crc32(data), len(data) & 0xFFFFFFFF)


cases = []


def add(name, blob, expect, **kw):
    cases.append({"name": name, "gz": blob.hex(), **expect, **kw})


def ok(data):
    return {"valid": True, "sha256": hashlib.sha256(data).hexdigest(), "length": len(data)}


strategies = {"default": zlib.Z_DEFAULT_STRATEGY, "filtered": zlib.Z_FILTERED, "huffman": zlib.Z_HUFFMAN_ONLY, "rle": zlib.Z_RLE, "fixed": zlib.Z_FIXED}
for kind in ["text", "repeat", "random", "zeros", "sitemap"]:
    for n in [0, 1, 7, 300, 4000, 70000, 200000]:
        if kind == "sitemap" and n in (0, 1, 7):
            continue
        d = sample(kind, n)
        for level in [0, 1, 6, 9]:
            for sname, strat in strategies.items():
                if sname != "default" and level not in (1, 9):
                    continue
                if n >= 70000 and (level not in (1, 9) or sname not in ("default", "fixed")):
                    continue
                # Incompressible bytes cost their full size in hex: keep a few small ones, and one
                # large stored/dynamic mix at level 6 added below.
                if kind == "random" and n >= 70000:
                    continue
                add(f"{kind}-{n}-L{level}-{sname}", member(d, level, strat), ok(d))
big_random = sample("random", 70000)
add("random-70000-L6-default", member(big_random, 6), ok(big_random))
d = sample("sitemap", 5000)
add("header-name-comment-extra-hcrc", member(d, 9, name="sitemap.xml", comment="made for the test", extra=b"\x01\x02\x03\x04", hcrc=True), ok(d))
add("header-extra-only", member(d, 6, extra=b"AB\x02\x00xy"), ok(d))
a, b = sample("text", 3000), sample("repeat", 2000)
add("two-members", member(a) + member(b), ok(a + b))
add("zero-padding", member(a) + b"\0" * 7, ok(a))
valid = member(sample("text", 6000), 9)
for cut in [3, 9, 11, 20, len(valid) // 2, len(valid) - 9, len(valid) - 8, len(valid) - 1]:
    add(f"truncated-at-{cut}", valid[:cut], {"valid": False})
for pos in [12, len(valid) // 3, len(valid) // 2, len(valid) - 12]:
    blob = bytearray(valid); blob[pos] ^= 0x10
    add(f"bit-flip-at-{pos}", bytes(blob), {"valid": False})
blob = bytearray(valid); blob[-8] ^= 1
add("wrong-crc", bytes(blob), {"valid": False})
blob = bytearray(valid); blob[-1] ^= 1
add("wrong-length", bytes(blob), {"valid": False})
blob = bytearray(valid); blob[2] = 7
add("bad-method", bytes(blob), {"valid": False})
add("trailing-junk", valid + b"junk", {"valid": False})
add("not-gzip", b"<?xml version='1.0'?><urlset/>", {"valid": False})
bomb = member(b"\0" * (64 * 1024 * 1024), 9)
add("bomb-64MiB-of-zeros", bomb, ok(b"\0" * (64 * 1024 * 1024)), bomb=True)

scrapy_partial = None
try:
    from scrapy.utils._compression import gunzip as scrapy_gunzip
    scrapy_partial = {}
    for c in cases:
        if c.get("bomb"):
            continue
        if not c["valid"]:
            try:
                out = scrapy_gunzip(bytes.fromhex(c["gz"]))
                c["scrapy"] = {"returned_bytes": len(out)}
            except Exception as e:  # noqa: BLE001
                c["scrapy"] = {"error": type(e).__name__}
except ImportError:
    pass
print(json.dumps({"class": "BENCHMARK_MEASUREMENT_NOT_ECDEV_RUNTIME_CAPTURE", "oracle": "CPython zlib/gzip (reference implementation)", "python": sys.version.split()[0],
                  "zlib": zlib.ZLIB_VERSION, "scrapy_commit": "54f7ed9cccbe19db3fe6cd2252bf3c29326c67d7", "cases": cases}))
