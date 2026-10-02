# .ynventa — Ynventa protocol v1

The canonical subsystem every Ynventa repository carries, byte-identical, at `.ynventa/`.
Rust, zero dependencies, no network, no runtime link to any other repository.

    cargo run --manifest-path .ynventa/Cargo.toml -- <command> [--root <repo>]

`status` `verify` `audit` `census` `extinction` `graph` `metrics` `compact` `conformance`
`migrate` `prove` `protocol` `fact` `knowledge` `organism` `donor` — identical semantics everywhere;
`help` lists them.

Knowledge (architecture, decisions, milestones, gaps) is typed facts, never Markdown:
`fact add <kind> <subject> <key> <value> --provenance <p>` writes one fact (a newer value of the
same kind/subject/key supersedes the older one, which is kept); `fact list` and
`knowledge view` read; `knowledge extract <doc>...` extracts exactly the named documents (and
licence texts). Extraction is lossless: a document's BLOCK facts (headings with their levels,
paragraphs, lists, code, tables, in order) and a legacy file's `content` rebuild it byte for
byte, proven against its digest; `knowledge view --document <path> [--outline]` rebuilds it.
A decision's status is normalised (`status`, with `status-text` when it says more) and its
relations are facts (`supersedes`, `superseded-by`, `refines`, … = the referenced ids).
`status` and `context` show the current milestones and decisions.

Authoritative state (repository-specific, never hand-written Markdown):

- `declared/{repository,donors,migration,technologies,organism}.rs` — typed Rust declarations:
  nodes (including developmental MATERIAL and, in Norl only, ORGAN nodes), edges, donors,
  capabilities (with `maps_to` and their Norl relevance), proofs, waves, shims, technologies and
  the organism (capabilities, backends and promotions of capabilities proven in another repository;
  empty outside Norl; a source's proof is a promotion's first rung, never Norl's growth). `migrate schema` rewrites
  declarations of an earlier grammar; the current reader never accepts them silently.
- `declared/universe/*.jsonl` — the repository's universe: every OSS repository, information
  source, authority, spec, paper, benchmark, dataset, simulator, standard and reference
  architecture it knows, one canonical record per line with its legacy text verbatim (the file
  rebuilds its legacy registry byte for byte). `migrate universe --from <registry>` imports one
  registry; `donor summary [--ecosystem <roots or capsules>...]` reports known universe, lifecycle
  layers, raw cross-repo references against deduplicated global donors, and every legacy
  registry not yet reconciled.
- `evidence/`, `history/`, `knowledge/` — content-addressed binary records (`*.ynv`).

Everything else in this directory is the canonical subsystem; `protocol.snapshot` pins it.
Generated views go to `target/ynventa/` and are never truth. The census reads committed
(indexed) content: `git add` before judging new files.
