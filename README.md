# ECDEV

Rust commerce engine with a local MCP/HTTP server, TypeScript dashboard, persisted runs and a source-backed donor registry. Development takes place directly on `main`.

The live-research wave is in progress. `domain/commerce/src/frontier.rs` implements a durable SQLite frontier with deterministic URL identity, fenced leases, crash recovery, priority, bounded retries, origin throttling and cancellation. Its six behavioral regression tests include forcibly terminating a worker after lease commit. Integration into the research workflow and Crawlee runtime oracle parity are still pending; the existing fixture proof is not a live proof.

Repository shape is checked by `tools/commerce/shape_gate.rs` using the unchanged canonical Ynventa assessor and ownership index. CI enforces canonical role roots and rejects architecture paths containing `crate` or `crates`. Cargo packages live directly in their declared graph nodes. The empty historical `crates` and `docs` roots have been removed. The separate upstream ECDEV registration proposal is recorded in `research/commerce/upstream-registration-action.json`; it has not been applied to canonical protocol bytes.

## Run

Requires Rust, Node.js and npm. From the repository root:

```sh
npm ci --prefix apps/web
npm run build --prefix apps/web
cargo run --locked --package ecdev-server --bin ecdev -- server
```

The server binds to loopback only. Dashboard: `http://127.0.0.1:8765/`; MCP: `http://127.0.0.1:8765/mcp`; health: `/health`; API: `/api/status`; events: `/events`. Set `ECDEV_PORT` to change the port. SQLite WAL and captured raw responses live in ignored `.ynventa/materialized/` directories. Run from the repository root, or set `ECDEV_ROOT` to its absolute path.

Register the running server with installed clients:

```sh
codex mcp add ecdev --url http://127.0.0.1:8765/mcp
claude mcp add --transport http ecdev http://127.0.0.1:8765/mcp
```

`ecdev stdio` uses the same engine. CLI commands include `doctor`, `status`, `providers`, `runs`, `runs inspect ID`, `runs replay ID`, and `call TOOL input.json`. Use `cargo run --locked --package ecdev-server --bin ecdev -- COMMAND` without installing the binary.

## Current behavior

There are 25 MCP tools. `ecdev.research.run` executes bounded native/public research from supplied seed URLs: DOM/JSON-LD extraction, source evidence, persistent cache, product candidates, economics from supplied assumptions, explicit rejections and a report. Fixture HTML is labeled `FIXTURE`. The dashboard exposes research, candidates, calls, budget, evidence edges and snapshot comparisons. Opportunity search separately stores an eleven-stage capability DAG and reports `PLAN_ONLY` / `UNAVAILABLE`; it does not manufacture market results. Economics records `SIMULATED` results. Replay reads persisted observations without network calls.

Paid budget defaults to zero. Semrush, Keepa and hosted crawling services are optional. `ecdev.product.analyze` accepts `{"market":"AMAZON_JP","asin":"B08N5WRWNW"}` (also `AMAZON_US`). Missing `KEEPA_API_KEY` returns `UNAVAILABLE`; configured credentials with zero budget return `DENIED_BUDGET` before IO. Paid acquisition requires explicit positive USD ceilings through all six variables: `ECDEV_PAID_PER_RUN_MINOR`, `ECDEV_PAID_PER_DAY_MINOR`, `ECDEV_PAID_PER_MONTH_MINOR`, `ECDEV_PAID_PER_PROVIDER_MINOR`, `ECDEV_PAID_PER_CAPABILITY_MINOR`, `ECDEV_PAID_REQUEST_CEILING_MINOR`. Reservations persist transactionally and retain their conservative ceiling when actual cost is unknown. These are operator-supplied ceilings, not provider price claims. The additional daily request bound is 100; `ECDEV_MAX_PROVIDER_REQUESTS_PER_DAY=0` disables acquisition. The native Keepa client makes one request without retries and stores successful raw bytes and evidence. Authentication and live credentialed behavior remain unverified.

Public fetching denies private/special addresses, credential URLs and unsupported content, pins validated DNS addresses, enforces conservative robots rules and bounds redirects, bytes, concurrency and pages. It captures content hashes and response validators. Cache TTL is one hour; expired captures can be revalidated with ETag/Last-Modified. `allow_stale=true` permits a capture up to one day beyond expiry after a fetch failure; stale evidence is labeled and its TTL is not renewed. Unknown freshness and monetary costs remain null. Research coverage at $0 is 9 of 14 explicitly declared stages (64.28%); the denominator and missing stages are exposed in `ecdev.provider.budget`. This measures executable stages, not completeness of market intelligence.

Other provider adapters, multi-provider opportunity execution, supplier matching, PPC acquisition, trained models and complete donor extinction are pending. Review `research/commerce/wave-report.json` for measured progress and remaining gates.

## Repository and Ynventa

The canonical subsystem is pinned byte-for-byte to [llgtrn/.Ynventa-](https://github.com/llgtrn/.Ynventa-) commit `c00a123c542b3c16fea4f82b45a5e544a1870178`. Typed declarations are authoritative. Code follows its role roots:

| Path | Responsibility |
| --- | --- |
| `domain/commerce` | Evidence types, economics, intent planner, provider boundary, durable runs |
| `adapter/keepa`, `adapter/web` | Optional Keepa IO and native public fetching/extraction |
| `apps/server`, `apps/web` | MCP/HTTP/events and their dashboard projections |
| `tools/commerce` | Full clone census, schemas, provenance and dependency registration |
| `tests/commerce` | Independent official MCP SDK integration client |
| `research/commerce` | Locked donor inventories, reviewed contracts and measured status |
| `.ynventa` | Unmodified canonical protocol and repository-specific declarations/evidence |

Twenty-five donors have verified remotes and full clones: ten initial seeds and fifteen new crawling/browser/Amazon/extraction donors. All 12,972 tracked files are inventoried; 77 classifications and 193 first-party parses remain unknown. There are 58,965 inventoried symbols. Six additional narrow contracts have source-reviewed capability mappings; other hypotheses remain unverified. Only Keepa's full semantic census is complete. Its current-value primitive matches 508 cases executed against the exact donor source; this is limited parity, not complete absorption. Seed donors remain research inputs. Thirteen runtime foundation packages from twelve upstreams remain explicitly registered external dependencies.

Canonical conformance has one upstream gap: ECDEV is absent from the closed shard registry. Preserve this failing gate until upstream registration exists. No donor is declared extinct and V1 is not achieved.

```sh
cargo run --manifest-path .ynventa/Cargo.toml -- protocol --check --root .
cargo run --manifest-path .ynventa/Cargo.toml -- conformance --root .
cargo run --manifest-path .ynventa/Cargo.toml -- extinction --root .
cargo run --manifest-path .ynventa/Cargo.toml -- prove --root .
```

New files must be staged before canonical indexed census/proofs. The full supplied specification is losslessly stored in `.ynventa/knowledge/8f30da7d287f8eae6dc86f6c0d286349.ynv`; reconstruct it with `knowledge view --document research/commerce/specification.md`. SHA-256: `6af066986bb05f97db7b345037e585d805eb64b0123a161dea5d076aa98f614c`.

The expansion request is also retained losslessly in `.ynventa/knowledge/a295263e64e42920349e7b59a1e46c20.ynv`; reconstruct with `knowledge view --document research/commerce/expansion-specification.md`. SHA-256: `6079de368bbd45bf3246cda7e058d8ecf15b2d511e2d16fc4d1b4a64b4f2d898`.

## Verify

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --manifest-path .ynventa/Cargo.toml -- --skip dogfood_repository_conforms
npm ci --prefix tests/commerce
node tests/commerce/mcp-smoke.mjs
```

The skipped canonical dogfood test requires upstream ECDEV shard registration; running the entire suite retains that real failure. No canonical test or implementation is patched. The smoke test requires the running server and zero configured paid budget; it makes no paid acquisition requests. Census tools require Python and pinned `tools/commerce/requirements.txt`. `expand.py` preserves reviewed seed records while acquiring the next wave. `study_expansion.py` records narrow reviewed source contracts. `validate.py` checks commit-level provenance; `--records-only` validates committed inventories when checkouts are absent. `foundation.py` is an initial generator and must not be rerun over reviewed records. `register_dependencies.py` requires installed dependencies and network access. Clones, build products and credentials are ignored by Git.
