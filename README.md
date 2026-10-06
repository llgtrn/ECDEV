# ECDEV

Rust commerce engine with a local MCP/HTTP server, TypeScript dashboard, persisted runs and a source-backed donor registry. Development takes place directly on `main`.

The live-research wave is in progress. `domain/commerce/src/frontier.rs` implements a durable SQLite frontier with deterministic URL identity, fenced leases, crash recovery, priority, bounded retries, origin throttling and cancellation. Research runs now use this frontier and can resume saved captures through `crawl_run_id` without reacquiring handled URLs. Six behavioral regressions include forcibly terminating a worker after lease commit. Crawlee runtime oracle parity remains pending.

ECDEV is a standalone repository. Its governance — repository graph, donor lifecycle, evidence, shape validator, extinction evaluator and ECDEV repository conformance — lives in `.ecdev/` and depends on no other repository. Repository shape is checked by `tools/commerce/shape_gate.rs` using the `.ecdev` assessor and ownership index. CI enforces canonical role roots and rejects architecture paths containing `crate` or `crates`. Cargo packages live directly in their declared graph nodes.

## Run

Requires Rust, Node.js and npm. From the repository root:

```sh
npm ci --prefix apps/web
npm run build --prefix apps/web
cargo run --locked --package ecdev-server --bin ecdev -- server
```

The server binds to loopback only. Dashboard: `http://127.0.0.1:8765/`; MCP: `http://127.0.0.1:8765/mcp`; health: `/health`; API: `/api/status`; events: `/events`. Set `ECDEV_PORT` to change the port. SQLite WAL and captured raw responses live in the ignored `.ecdev-data/` directory. Run from the repository root, or set `ECDEV_ROOT` to its absolute path.

Register the running server with installed clients:

```sh
codex mcp add ecdev --url http://127.0.0.1:8765/mcp
claude mcp add --transport http ecdev http://127.0.0.1:8765/mcp
```

`ecdev stdio` uses the same engine. CLI commands include `doctor`, `doctor sp-api`, `status`, `providers`, `runs`, `runs inspect ID`, `runs replay ID`, and `call TOOL input.json` and `governance COMMAND`. Use `cargo run --locked --package ecdev-server --bin ecdev -- COMMAND` without installing the binary.

## Current behavior

There are 40 MCP tools. `ecdev.trend.hypothesize` turns a frozen trend snapshot into product hypotheses, unverified candidate links and ranked zero-paid research actions; social evidence never shortlists and trend is never treated as demand. Public supplier leads retain page assertions and unknown terms. `ecdev.research.run` accepts `follow_up_run_id` to execute bounded public actions selected by a disclosed information-gain heuristic; rejected candidates are skipped. It reads supplier pages without submitting contact forms. Per-run completeness separates live IO, fixture and cached observations, with explicit field and stage denominators. `ecdev.research.run` executes bounded native/public research from supplied seed URLs: DOM/JSON-LD/microdata extraction, source evidence, persistent cache, product candidates, economics from supplied assumptions, explicit rejections and a report. Fixture HTML is labeled `FIXTURE`. The dashboard exposes research, candidates, calls, budget, evidence edges, snapshot comparisons and persisted public watches. Watches use fresh bounded acquisitions, lease recovery, and price, availability, product-data and confirmed 404/410 disappearance triggers; blocked or unknown captures preserve the prior baseline. `ecdev.monitor.create` accepts `watch_id` to update and `enabled:false` to disable. Discovery, inspection, comparison and provider status also have high-level MCP calls. Opportunity search separately stores an eleven-stage capability DAG and reports `PLAN_ONLY` / `UNAVAILABLE`; it does not manufacture market results. Economics records `SIMULATED` results. Replay reads persisted observations without network calls.

The recorded JP live acquisition discovered 176 URLs, captured 99 pages, found Product metadata on 95 pages and created 164 candidate observations. It rejected 62 observations against supplied price bounds, with zero paid calls and zero paid cost. The subsequent field-provenance and identity pass resolves 164 observations into 95 candidates, retains title-only similarities as uncertain links, and keeps a mug and its replacement part separate despite their shared manufacturer number. None were shortlisted. The old HARIO seed remained blocked; its accessible canonical catalog supplied the captures. `research/commerce/live-jp-acquisition-proof.json` records raw capture hashes, blocked routes, frontier state and measured field observability. `research/commerce/live-jp-normalization-proof.json` separately records the corrected CACHED reassessment of the same live captures with zero new network calls. Listing counts and visibility proxies describe only the captured sample. Unknown costs leave expected profit unknown; reported monetary upper bounds are not forecasts. The broader wave remains incomplete; see `research/commerce/live-wave-progress.json`.

The official SP-API boundary is credential-ready but unvalidated live: `ecdev doctor sp-api` (MCP `ecdev.provider.doctor`) reports credential presence without reading values, the operator gate, request budget, endpoint and token state, and per-operation availability. Without `SP_API_CLIENT_ID`, `SP_API_CLIENT_SECRET` and `SP_API_REFRESH_TOKEN` it reports `LIVE_AUTH_BLOCKED_BY_CREDENTIALS`. Catalog, pricing, fees, listings, FBA inventory summaries, marketplace participations and product type definitions have read-only request/response boundaries tested against official model examples, and the seller-scoped reads run through the engine as `ecdev.seller.read` with the same gate, budget and persistence; orders/PII and restricted-data-token operations are a separate disabled domain, and writes are disabled. See `research/commerce/sp-api-credential-readiness.json`.

Paid budget defaults to zero. Semrush, Keepa and hosted crawling services are optional. `ecdev.product.analyze` accepts `{"market":"AMAZON_JP","asin":"B08N5WRWNW"}` (also `AMAZON_US`). Missing `KEEPA_API_KEY` returns `UNAVAILABLE`; configured credentials with zero budget return `DENIED_BUDGET` before IO. Paid acquisition requires explicit positive USD ceilings through all six variables: `ECDEV_PAID_PER_RUN_MINOR`, `ECDEV_PAID_PER_DAY_MINOR`, `ECDEV_PAID_PER_MONTH_MINOR`, `ECDEV_PAID_PER_PROVIDER_MINOR`, `ECDEV_PAID_PER_CAPABILITY_MINOR`, `ECDEV_PAID_REQUEST_CEILING_MINOR`. Reservations persist transactionally and retain their conservative ceiling when actual cost is unknown. These are operator-supplied ceilings, not provider price claims. The additional daily request bound is 100; `ECDEV_MAX_PROVIDER_REQUESTS_PER_DAY=0` disables acquisition. The native Keepa client makes one request without retries and stores successful raw bytes and evidence. Authentication and live credentialed behavior remain unverified.

Public fetching denies private/special addresses, credential URLs and unsupported content, pins validated DNS addresses, enforces conservative robots rules and bounds redirects, bytes, concurrency and pages. It captures content hashes and response validators. Cache TTL is one hour; expired captures can be revalidated with ETag/Last-Modified. `allow_stale=true` permits a capture up to one day beyond expiry after a fetch failure; stale evidence is labeled and its TTL is not renewed. Unknown freshness and monetary costs remain null. Research coverage at $0 is 10 of 14 explicitly declared stages (71.42%); the denominator and missing stages are exposed in `ecdev.provider.budget`. This measures executable stages, not completeness of market intelligence.

Other provider adapters, multi-provider opportunity execution, supplier matching, PPC acquisition, trained models and extinction of the remaining donors are pending. Review `research/commerce/wave-report.json` for measured progress and remaining gates.

## Repository and governance

`.ecdev/` is ECDEV's own governance authority (see `.ecdev/README.md`). Typed declarations are authoritative. Code follows its role roots:

| Path | Responsibility |
| --- | --- |
| `domain/commerce` | Evidence types, economics, intent planner, provider boundary, durable runs |
| `adapter/keepa`, `adapter/web`, `adapter/marketplace` | Optional Keepa IO, native public fetching/extraction, official SP-API boundary |
| `apps/server`, `apps/web` | MCP/HTTP/events and their dashboard projections |
| `tools/commerce` | Full clone census, schemas, provenance, shape gate and governance snapshots |
| `tests/commerce` | Independent official MCP SDK integration client |
| `research/commerce` | Locked donor inventories, reviewed contracts and measured status |
| `.ecdev` | ECDEV governance: declarations, evidence, lifecycle and repository conformance |

The recorded lifecycle and ECDEV repository conformance are in `research/commerce/governance/` (regenerate with `python tools/commerce/governance_snapshot.py`). The governance migration away from the earlier external subsystem is recorded in `research/commerce/governance/migration.json`.

```sh
cargo run --manifest-path .ecdev/Cargo.toml -- conformance --root .
cargo run --manifest-path .ecdev/Cargo.toml -- extinction --root .
cargo run --manifest-path .ecdev/Cargo.toml -- prove --root .
```

New files must be staged before the indexed census and proofs. The full supplied specification is stored losslessly in `.ecdev/knowledge`; reconstruct it with `knowledge view --document research/commerce/specification.md` (SHA-256 `6af066986bb05f97db7b345037e585d805eb64b0123a161dea5d076aa98f614c`), and the expansion request with `knowledge view --document research/commerce/expansion-specification.md` (SHA-256 `6079de368bbd45bf3246cda7e058d8ecf15b2d511e2d16fc4d1b4a64b4f2d898`).

## Verify

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --manifest-path .ecdev/Cargo.toml
npm ci --prefix tests/commerce
node tests/commerce/mcp-smoke.mjs
```

The governance suite includes ECDEV self-conformance and runs in full. The smoke test requires the running server and zero configured paid budget; it makes no paid acquisition requests. Census tools require Python and pinned `tools/commerce/requirements.txt`. `expand.py` preserves reviewed seed records while acquiring the next wave. `validate.py` checks commit-level provenance; `--records-only` validates committed inventories when checkouts are absent. One-shot generators that edited declarations in the retired grammar were removed; declarations change through typed edits and `ecdev-gov`. Clones, build products and credentials are ignored by Git.

Native microdata extraction matches 132 cases executed against the locked extruct parser. Nested scopes, item references, repeated properties, value tags, URL resolution and cleaned text are covered. Raw microdata graph pointers remain inspectable in candidate field evidence. Conflicting JSON-LD and microdata prices clear the combined price instead of selecting a preferred format. Extruct is a research-only oracle; full donor absorption remains unproven.

The native frontier now matches 46 locked Crawlee queue traces (564 operations) for deduplication, FIFO/forefront ordering, reclaim, handled-state retention, lease expiry and shared reopen. Regular retries rejoin behind waiting work at equal priority. Lease tokens and retry budgets remain enforced. The research-only harness executes unchanged donor TypeScript with its integrity-pinned native backend; it is not a production dependency. Regenerate with `npm ci` then `npm run oracle:crawlee` in `tools/commerce` after restoring both locked donor checkouts. Full crawler error policies, dynamic autoscaling and the donor normalization API remain unproven.

Public HTML decoding follows BOM, HTTP charset, the first 1,024 bytes of HTML meta declarations, then strict UTF-8. Original response bytes remain the raw capture and every field annotation uses their hash; the decoding recipe also records the decoded UTF-8 hash. Invalid or unsupported encodings remain unavailable. The native policy matches 54 outputs executed against locked w3lib 2.5.0 for valid declared encodings and BOM precedence. Statistical guessing, replacement of malformed bytes, UTF-32, and full Scrapy parity are unproven. A fresh zero-paid HARIO manufacturer capture produced one public lead with unknown commercial terms; that response was UTF-8, so live legacy-charset acquisition is not claimed. See `research/commerce/document-decoding-proof.json`.

Native formatted-number normalization uses exact decimal strings and disclosed separator heuristics, with an explicit supported currency required for minor units. `scrapinghub/price-parser` is ECDEV's first EXTINCT donor: all seven capabilities ECDEV relies on are native with fresh parity and regression proofs over 21,535 frozen oracle cases, six donor surfaces ECDEV cannot invoke are justified in `research/commerce/price-parser-exclusions.json`, and no import, build, test or resident edge remains (`research/commerce/price-parser-extinction.json`). The retired oracle generator is kept in history only. Graphiti's deterministic evidence-memory semantics (MinHash/LSH name-similarity candidates, bi-temporal fact supersession, reciprocal rank fusion) are native in `domain/commerce/src/memory.rs` and match 1,580 frozen oracle cases; its module-level semantic matrix is `research/commerce/graphiti-semantic-matrix.json`. Watches now record observed price and availability as validity-windowed facts (`facts` in `ecdev.monitor.status`), where a newer value closes the window it supersedes, and near-duplicate titles become uncertain `POSSIBLE_EQUIVALENCE` links that are never merged. Standalone explicit OpenGraph products can now reach candidates. Page metadata enriches a single product with raw field locators; multiple-product association is deferred. Canonical URLs, declared original prices, derived discounts, JSON-LD breadcrumbs and offer shipping assertions retain provenance. Conflicting prices remain unselected, and entity resolution preserves derived evidence status. A fresh Japan run discovered 152 URLs, captured four pages and normalized three candidates at zero paid cost; canonical URLs were observed, while supplier terms and a supported shortlist remained unavailable. See `research/commerce/price-page-proof.json`.

JSON-LD numeric prices retain exact decimal precision through parsing. Unsupported fractional minor units remain unknown instead of rounding to a valid price. Numeric exponent expansion is bounded; original spelling remains in the hashed HTML capture. See `research/commerce/numeric-price-precision-proof.json` for the reproduced rounding error and regression scope.

A fresh JP public research run now reaches a shortlist for further research: 332 discovered URLs, 40 captures, 39 product pages, 39 normalized candidates, three price-constraint rejections, 35 insufficient-evidence candidates and one shortlist entry at zero paid cost. Matching GTIN assertions link manufacturer and retailer offers; source pointers, capture hashes and zero-I/O replay are verified in `research/commerce/live-shortlist-proof.json`. The policy requires two captured listing origins, a stable positive price and an available offer. Different origins do not establish independent publishers. Unknown supplier costs, expected profit, demand and regulatory validation remain unknown. Optional observed weight/category constraints are supported; missing constrained fields block selection.

Public Amazon HTML acquisition now has its own public-amazon provider and PUBLIC_AMAZON source layer, separate from OFFICIAL_SP_API and KEEPA. It enforces native robots policy and redirect host scope, preserves requested and final URLs, and records URL-derived ASIN assertions without claiming official validation. Cache and restart recovery preserve provider identity. The live Amazon JP search attempt returned HTTP 503; two native retailer/manufacturer captures still reached a further-research shortlist at zero paid cost. Amazon product extraction remains fixture verified, and SP-API acquisition remains unimplemented. See research/commerce/public-amazon-proof.json.

System status and the dashboard now show historical zero-paid shortlist evidence after verifying acquired capture hashes from two listing origins. Fixture, cached and replay runs cannot establish this status; missing or altered raw captures invalidate it. The official SDK smoke test reports its fixture scope separately. The eight requested donors have a locked-license and canonical lifecycle review, with no full absorption or extinction claim. See research/commerce/live-status-proof.json, research/commerce/donor-lifecycle-review.json, and the measured requirement report in research/commerce/live-wave-report.json.

Native HTTP response failures now retain status, known request counts and Retry-After headers. Delays reach the persistent frontier and its run-scoped origin cooldown; stale-cache completion can preserve that cooldown. Seconds and three HTTP-date forms have regression coverage. A fresh live Amazon request returned HTTP 503 without Retry-After; its two requests are now counted alongside four fallback requests, with one further-research shortlist and zero paid cost. Timing remains regression evidence. Redirect-specific deferral and broader origin coordination remain pending. See research/commerce/http-retry-proof.json.

Published organization offers now retain JSON-LD and microdata term provenance, exact unit-price specifications, eligible-quantity lower bounds, delivery lead time and source claims. Offers and publishers stay separate; retail prices and unverified quotations never become product costs. A fresh three-page supplier pass used six HTTP requests at zero paid cost and found no structured commercial terms. See `research/commerce/supplier-terms-proof.json`.

Competition reports now retain per-product-assertion availability, review, shipping and variation evidence with a fixed 24-field completeness denominator and heuristic metadata support. Multiple assertions may share one captured listing page; listing-page counts remain deduplicated. Brand-label proportions disclose all-candidate and known-label denominators, with ownership and market share unverified. The candidate detail dashboard includes a listing evidence table. A fresh two-page run made four HTTP requests at zero paid cost; a subsequent cached projection made no new requests. See `research/commerce/listing-quality-proof.json`. Browser visual verification was unavailable because automation kernel assets failed to load; the TypeScript build and served assets were verified.
