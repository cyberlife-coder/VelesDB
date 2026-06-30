# Changelog — velesdb-memory

All notable changes to the `velesdb-memory` crate are documented here. This
crate is versioned independently of the VelesDB workspace (0.x cadence) and is
released on its own `velesdb-memory-vX.Y.Z` tag.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.1] - 2026-06-30

### Security

- **Upgraded `rmcp` 1.8.0 → 2.0.0**, which patches three advisories: OAuth token
  spoofing, SSRF via crafted MCP URLs, and a session-id leak in error responses.
  No code changes were needed — the MCP server/macros API stayed compatible.

### Fixed

- **`truncate()` UTF-8 panic** (extract error previews): the budget is now checked
  *before* appending a word, dropping the post-hoc `String::truncate` that could
  panic when the 120-byte limit fell mid-multibyte-character.
- **Dead code in `validate_relation`**: removed the redundant `is_ascii()` guard
  (`char::is_ascii_control()` is already `false` for non-ASCII code points).

## [0.3.0] - 2026-06-30

### Added

- **Durable TTL on `remember`.** Facts can now expire. `remember` (MCP tool) and
  `MemoryService::remember_with_ttl` take an optional `ttl_seconds`; the expiry is
  persisted with the fact (`_veles_expires_at`), so it survives a restart, and
  expired facts stop being recalled. Metadata and a TTL combine. Set a server-wide
  default with `VELESDB_MEMORY_DEFAULT_TTL` (seconds); `0` means permanent. The
  Node binding's `remember` gains the matching `ttlSeconds` argument.

### Fixed

- **Cleaner MCP tool schemas.** Stripped `schemars`' non-standard integer `format`
  keywords (`uint64`/`uint`) from the generated tool schemas, so strict MCP clients
  no longer log `unknown format "uint64" ignored` for every id field.

## [0.2.0] - 2026-06-29

Benchmark milestone: the tri-engine is no longer just *wired* — each leg is
*measured* to beat pure-vector retrieval on its specialty, generation-free, on
public/real data, and the engines are shown to compound.

### Added
- **Generation-free retrieval benchmarks** isolating each engine's contribution,
  reproducible from bundled examples (`examples/{multihop,timeqa,colfilter,triengine}`):
  - **Graph (`why()` BFS) — multi-hop supporting-fact recall.** On **HotpotQA**
    (3 000 dev, distractor) fused vector+graph lifts supporting-fact recall
    **+3.3pp** overall and **+5.6pp** on retrieving *both* bridge facts, with an
    idf-weighted bridge that suppresses the flooding a naive boost causes. The
    win **replicates on a second independent dataset, 2WikiMultiHopQA**,
    concentrated on the genuinely multi-hop question types.
  - **ColumnStore (`recall_where` numeric range) — time-scoped recall.** On real
    **TimeQA** Wikipedia bios, the year-range predicate lifts gold-sentence recall
    **+9.7pp** (+18.6pp on a controlled synthetic pilot) where cosine alone cannot
    disambiguate candidates that differ only by a number.
  - **Tri-engine compounding capstone** (`examples/triengine`): on a task that is
    multi-hop *and* time-scoped at once, Graph and ColumnStore together lift recall
    more than the sum of their individual gains — the engines stack.
- **LoCoMo harness** (`examples/locomo/`) extended into a tuning/diagnostic
  workbench: retrieval-only and explanation modes, per-category diagnostics, BM25
  baseline, idf-weighted graph fusion, date-context/date-routing and a temporal
  scaffold, an optional Claude judge/generator, and a configurable evidence budget.
- Positioning and benchmark write-ups (`POSITIONING.md`, `BENCHMARK.md`) grounding
  every claim in a reproducible measurement, with each engine's honest limit
  disclosed.

### Notes
- No public API change — this release adds benchmarks, examples and documentation
  around the existing `MemoryService` / MCP surface introduced in 0.1.0.

## [0.1.0] - Unreleased

First release of the local-first MCP memory server for AI agents.

### Added
- MCP tools over stdio mapping onto VelesDB's in-core Agent Memory SDK:
  `remember`, `recall`, `recall_where` (fused vector + ColumnStore range/filter
  recall), `relate`, `forget`, `why` (vector recall + multi-hop graph traversal —
  the connected-subgraph differentiator), and `remember_extracted` (auto text →
  fact↔topic graph via an `Extractor`).
- The same high-level `MemoryService` is consumed beyond the MCP server by the
  Python binding (`velesdb-python`) and the Node.js binding (`velesdb-node` /
  `@wiscale/velesdb-memory-node`); the library is feature-gated (`default-features
  = false` drops the rmcp/tokio MCP stack) so bindings link a lean core.
- `recall_where` activates a secondary bitmap-prefilter index on first use, so
  filtered recall stays flat as the collection grows (instead of an O(n) scan).
- Pluggable embeddings: a deterministic, offline `HashEmbedder` by default and an
  optional on-device `OllamaEmbedder` (`--features ollama`).
- Structured metadata (ColumnStore facet) with exact-match filtering on `recall`
  and `why`.
- Input guards (max fact size, capped recall limit and hop depth) and clean
  MCP error-code mapping: client-input errors map to `invalid_params`, faults to
  `internal_error`. `relate` validates both endpoints exist up front, so an
  unknown id is reported as `invalid_params` (not an internal fault) and the
  graph never gains an edge dangling off an unstored memory.
- License boundary by construction: memory semantics only, never raw database
  capabilities.

[0.3.1]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.3.1
[0.3.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.3.0
[0.2.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.2.0
[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.1.0
