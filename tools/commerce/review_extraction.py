"""Record immutable commerce extraction studies; donor source is research evidence only."""
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
AREA = ROOT / "research/commerce"

# Contracts are reviewed against implementation, with deliberate native differences recorded.
REVIEWS = [
    ("mozilla--readability", "Readability.js", "_getArticleMetadata(jsonld)",
     "Metadata names are trimmed, case-folded and dotted DC names normalized. Title/excerpt precedence includes JSON-LD, Dublin Core, OpenGraph, standard and Twitter metadata. Empty content is ignored and entities decoded. The donor overwrites repeated names and selects a preferred source; ECDEV preserves individual assertions and conflicts instead.",
     ["test/test-readability.js", "test/test-pages/003-metadata-preferred/source.html", "test/test-pages/003-metadata-preferred/expected-metadata.json"],
     ["adapter/web/src/commerce.rs", "adapter/web/src/page_product.rs"],
     ["named_metadata_preserves_repeated_claims_conflicts_and_exact_locators"]),
    ("mozilla--readability", "Readability.js", "_getArticleTitle()",
     "Document title supplies a fallback; donor separator and heading heuristics target article titles. ECDEV keeps the complete decoded document title, only when a single identified product lacks a title assertion, and marks it DERIVED. It does not strip seller suffixes or turn an article into a product.",
     ["test/test-readability.js"], ["adapter/web/src/page_product.rs"],
     ["document_title_fallback_requires_single_product_and_remains_derived"]),
    ("firecrawl--firecrawl", "apps/api/src/scraper/scrapeURL/lib/extractMetadata.ts", "export async function extractMetadata(",
     "Rust metadata extraction is attempted first and Cheerio used on failure. Standard description and OpenGraph/DC fields are extracted, custom repeated metadata accumulated; descriptions concatenate while other duplicates become arrays. ECDEV absorbs deterministic metadata assertion collection, retaining exact tag/attribute locators and contradictory values, without hosted Firecrawl, an LLM, concatenated fact selection or its runtime package.",
     ["apps/api/src/__tests__/snips/metadata-concat.test.ts"],
     ["adapter/web/src/commerce.rs", "adapter/web/src/page_product.rs"],
     ["named_metadata_preserves_repeated_claims_conflicts_and_exact_locators"]),
    ("scrapinghub--extruct", "extruct/_extruct.py", "def extract(",
     "Syntax and error policy validation precede DOM construction; independent processors preserve structured graphs, optionally normalized. ECDEV preserves JSON-LD, microdata and OpenGraph evidence independently before commerce field projection; malformed JSON errors and conflicts remain visible. RDFa, Microformats, donor repair and optional output modes are outside this replacement.",
     ["tests/test_opengraph.py", "tests/test_uniform.py"],
     ["adapter/web/src/lib.rs", "adapter/web/src/commerce.rs", "adapter/web/src/page_product.rs"],
     ["microdata_products_and_cross_format_conflicts_retain_evidence", "standalone_meta_and_cross_format_conflicts_are_explicit"]),
    ("scrapinghub--extruct", "extruct/w3cmicrodata.py", "class LxmlMicrodataExtractor",
     "Nested scopes, itemrefs, repeated properties, HTML value tags, relative URLs and cleaned text produce a graph. ECDEV preserves original graph pointers and projects each identified Product separately; the bounded 132-case independent oracle is separate from this source study.",
     ["tests/test_microdata.py"], ["adapter/web/src/microdata.rs", "adapter/web/src/lib.rs"],
     ["locked_extruct_microdata_oracle"]),
    ("scrapinghub--price-parser", "price_parser/parser.py", "def fromstring(",
     "Currency recognition, text selection and decimal conversion are distinct stages. Donor parsing accepts a currency hint, explicit separators and free text. ECDEV only converts explicit price assertions with a supported unambiguous currency into exact minor units; it preserves raw numeric lexemes and refuses lossy fractions.",
     ["tests/test_price_parsing.py"], ["adapter/web/src/price.rs", "adapter/web/src/lib.rs"],
     ["precision_and_ambiguous_currency_do_not_invent_money", "json_numeric_prices_preserve_source_precision"]),
    ("scrapinghub--price-parser", "price_parser/parser.py", "def extract_price_text(",
     "The donor selects the first numeric substring, rejects percentage-only strings, handles euro-as-decimal and treats Free as zero. ECDEV deliberately does not select a price from arbitrary marketing text or infer a free product. Explicit structured prices are parsed by the separately oracle-tested numeric kernel.",
     ["tests/test_price_parsing.py"], ["adapter/web/src/price.rs"],
     ["locked_price_parser_numeric_oracle"]),
    ("scrapy--scrapy", "scrapy/http/response/text.py", "def text(self)",
     "Response text consults encoding before caching decoded body and delegates to w3lib html_to_unicode. Header/body/BOM and inferred encoding are distinct. ECDEV keeps original wire hashes, uses deterministic declared encoding precedence and fails on unsupported or invalid data; statistical autodetection/replacement is not claimed. The bounded w3lib oracle is separate evidence.",
     ["tests/test_http_response_text.py"], ["adapter/web/src/document.rs", "adapter/web/src/lib.rs"],
     ["locked_w3lib_declared_document_oracle", "declared_japanese_preserves_wire_provenance"]),
    ("scrapy--scrapy", "scrapy/http/response/text.py", "def selector(self)",
     "A response memoizes its selector and CSS/XPath delegate through it. ECDEV owns HTML5 DOM selector traversal and retains field-level selector/match-index or structured graph pointers rather than flattening extracted values without their origin. XPath and Scrapy selector API parity are not claimed.",
     ["tests/test_selector.py", "tests/test_http_response_text.py"],
     ["adapter/web/src/commerce.rs", "adapter/web/src/lib.rs", "adapter/web/src/page_product.rs"],
     ["named_metadata_preserves_repeated_claims_conflicts_and_exact_locators"]),
]


def immutable_file(checkout, commit, path):
    args = ["git", "-c", f"safe.directory={checkout.as_posix()}", "-C", str(checkout)]
    data = subprocess.check_output(args + ["show", f"{commit}:{path}"])
    blob = subprocess.check_output(args + ["rev-parse", f"{commit}:{path}"]).decode().strip()
    return data, blob


def main():
    registry = {d["donor_id"]: d for d in json.loads((AREA / "donors/registry.json").read_text(encoding="utf-8"))["donors"]}
    contracts = []
    for donor, path, symbol, contract, tests, native, native_tests in REVIEWS:
        identity = registry[donor]
        commit = identity["commit_sha"]
        checkout = AREA / "donors/checkouts" / donor
        data, blob = immutable_file(checkout, commit, path)
        source = data.decode("utf-8")
        assert symbol in source, (donor, path, symbol)
        line = source[:source.index(symbol)].count("\n") + 1
        test_files = []
        for test in tests:
            test_data, test_blob = immutable_file(checkout, commit, test)
            test_files.append({"source_path": test, "blob_hash": test_blob, "sha256": hashlib.sha256(test_data).hexdigest(), "execution": "LOCATED_AND_REVIEWED_NOT_DONOR_SUITE_EXECUTION"})
        for file in native:
            assert (ROOT / file).is_file(), file
        for name in native_tests:
            assert any(name in (ROOT / file).read_text(encoding="utf-8") for file in native + ["apps/server/src/lib.rs"]), name
        contracts.append({"donor_id": donor, "commit_sha": commit, "source_path": path, "blob_hash": blob, "content_sha256": hashlib.sha256(data).hexdigest(), "symbol": symbol, "line_start": line, "reviewed_excerpt": "\n".join(source.splitlines()[line-1:line+14]), "behavior_contract": contract, "donor_test_files": test_files, "native_sources": native, "native_regressions": native_tests, "status": "SOURCE_REVIEWED_NATIVE_COMMERCE_SUBSET", "oracle_parity": "NOT_CLAIMED_BY_THIS_STUDY"})
    result = {"status": "FIVE_NAMED_EXTRACTION_DONORS_STUDIED", "required_wave_section": 7, "contracts": contracts, "distinct_donors": len({c["donor_id"] for c in contracts}), "runtime_donor_dependencies_added": 0, "oracle_families_added": 0, "scope": "Relevant commerce extraction behavior; intentional conservative differences explicit. No full article extraction, hosted scrape, whole-donor parity or extinction claim.", "metadata_cache_schema": 12}
    (AREA / "extraction-source-review.json").write_text(json.dumps(result, ensure_ascii=True, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"status": result["status"], "contracts": len(contracts), "distinct_donors": result["distinct_donors"]}))


if __name__ == "__main__":
    main()
