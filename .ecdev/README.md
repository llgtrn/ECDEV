# .ecdev — ECDEV governance

ECDEV's own governance authority: repository graph, node ownership, donor registry and
lifecycle, evidence store, shape validator, extinction evaluator, technology/capability
declarations and **ECDEV repository conformance**. Rust, zero dependencies, no network, no
dependency on any other repository.

    cargo run --manifest-path .ecdev/Cargo.toml -- <command> [--root <repo>] [--json]

`status` `verify` `audit` `census` `extinction` `graph` `metrics` `compact` `conformance`
`migrate` `prove` `protocol` `fact` `knowledge` `donor` `context` `show` `backlinks`
`technology`; `help` lists them. The server exposes the same binary as `ecdev governance <command>`.

Authoritative state (never hand-written Markdown):

- `declared/{repository,donors,migration,technologies}.rs` — typed Rust declarations checked by
  rustc and read identically at run time: nodes, edges, donors, capabilities (with `maps_to`,
  `replacement`, proofs and `relevance`), cutovers, waves, shims and technologies.
- `declared/universe/*.jsonl` — every repository and source ECDEV knows, one canonical record per
  line, rebuilding `research/commerce/donors/registry.json` byte for byte.
- `evidence/`, `history/`, `knowledge/` — content-addressed binary records (`*.ecg`).

Donor lifecycle (effective states are computed from evidence; a declared state is only a claim):
`DISCOVERED → REGISTERED → CENSUSED → TECHNOLOGY_MAPPED → SPECIFIED → NATIVE_TARGETED →
NATIVE_SHADOW → PARITY_PROVEN → CAPABILITY_RELEVANCE_RESOLVED → CUTOVER → EXTINCT`.
Every declared capability is either `Relevance::ReliedOn(..)` (ECDEV calls it: `required`, must
be native and proven) or `Relevance::NotRelevant(..)` (ECDEV can never invoke it: not
`required`, its `spec` names a tracked exclusion review). EXTINCT requires every gate: no
runtime/build/linked/test edge, no import, no resident donor source, native canonical
replacements with fresh parity and regression proofs, full mapping, resolved relevance, a
declared cutover, and no node depending on the donor.

Generated views go to `target/ecdev-governance/` and are never truth. The census reads committed
(indexed) content: `git add` before judging new files. Runtime data of the server lives in the
ignored `.ecdev-data/`, never here.
