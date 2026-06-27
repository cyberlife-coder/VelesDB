# Changelog — velesdb-memory

All notable changes to the `velesdb-memory` crate are documented here. This
crate is versioned independently of the VelesDB workspace (0.x cadence) and is
released on its own `velesdb-memory-vX.Y.Z` tag.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/cyberlife-coder/VelesDB/releases/tag/velesdb-memory-v0.1.0
