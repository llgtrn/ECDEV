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
Two independent questions are asked of every donor capability.

- **Runtime adoption** (`relevance`): `Relevance::ReliedOn(..)` (production calls it: `required`,
  must be native and proven) or `Relevance::NotAdopted(..)` (production does not execute it: not
  `required`, its `spec` names a tracked review). A donor whose runtime was evaluated and never
  adopted carries `Exception::Rejected`, reported as `REJECT_RUNTIME`.
- **Knowledge value** (`knowledge`, mandatory): `Unreviewed`, `StudyCandidate`,
  `BenchmarkCandidate`, `AlgorithmCandidate` (open), or `ReferenceOnly`, `Absorbed`,
  `IndependentNative`, `Divergent`, `NoResearchValue(ResearchScope::<ground>, ..)` (resolved).
  `NoResearchValue` is only admissible on a scope ground (frontend, release tooling, assets,
  vendor telemetry, hosted-service plumbing, language bindings, unrelated domain); "no current
  caller", "a different native implementation", "a restrictive licence" or "not evidence" are
  rejected by `knowledge.irrelevance_admissible` — they constrain adoption, never knowledge. An
  unrelated-domain closure whose own reason names an ECDEV target domain (market, product,
  demand, competition, supplier, sourcing, economics, PPC, listing, seller operations,
  monitoring, trends, memory/evidence, simulation, decision support, learning) is refused too:
  "yes or uncertain" keeps the question open.

A runtime-rejected donor stays in the knowledge universe; its donor knowledge state is
`UNREVIEWED`, `STRUCTURAL_CENSUS`, `SEMANTIC_CENSUS` (every extracted capability decided, but the
census whole-source `semantic_review` is not `COMPLETE`), `ACTIVE_STUDY` or `STUDY_COMPLETE`
(semantic review `COMPLETE` and every capability resolved). Production metrics
(`production_capabilities_*`) and knowledge metrics (`known_donor_capabilities`,
`semantic_censused_capabilities`, `unreviewed_capabilities`, `not_relevant_after_review`,
`research_questions_open`, `knowledge_coverage_ratio`, `donors_semantically_censused`,
`donors_with_open_candidates`, ...) are reported separately: 100 % production completion never
means the known donor universe is studied.

The knowledge denominator never shrinks. `census --record` stores every known
`<donor>/<capability>` key in `.ecdev/history/`; a recorded key that is no longer declared counts
as `capabilities_withdrawn` (known and open), raises `CAPABILITY_WITHDRAWN`, blocks the donor's
study and extinction, and fails `knowledge.denominator_preserved` — unless a declared capability of
the same donor lists it in `absorbs` (a rename or split).

Licence policy is three separate answers read off the declared licence string (`src/licence.rs`;
a governance policy, not a legal conclusion): code adoption (`ADOPT_WITH_NOTICE`,
`NOT_COPIED_WITHOUT_REVIEW`, `NOT_COPIED`, `NOT_COPIED_UNTIL_VERIFIED`), knowledge study
(`STUDY_AND_DERIVE` or `STUDY_READ_ONLY` — never forbidden), and native reimplementation
(`DERIVED_WITH_ATTRIBUTION` or `CLEAN_ROOM_ONLY`). `licence.derivation_admissible` refuses an
`Absorbed` (derived-native) capability of a clean-room-only donor.

EXTINCT requires every gate: no runtime/build/linked/test edge, no import, no resident donor
source, native canonical replacements with fresh parity and regression proofs, full mapping,
resolved adoption, resolved knowledge (`KNOWLEDGE_RESOLVED`: extinction removes a dependency only
after its knowledge is learned), a declared cutover, and no node depending on the donor.

Generated views go to `target/ecdev-governance/` and are never truth. The census reads committed
(indexed) content: `git add` before judging new files. Runtime data of the server lives in the
ignored `.ecdev-data/`, never here.
