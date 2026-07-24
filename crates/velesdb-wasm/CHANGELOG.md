# Changelog

All notable changes to `velesdb-wasm` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **BREAKING (behavior) — `weighted` fusion default weights**: the default
  weights used by `multi_query_search(..., strategy: "weighted")` when no
  explicit weights are supplied changed from the WASM-local, non-overridable
  `avg=0.5, max=0.3, hit=0.2` to `velesdb-core`'s canonical
  `DEFAULT_WEIGHTED_*` constants, `avg=0.6, max=0.3, hit=0.1` — the same
  defaults already used by the Python bindings and the `velesdb_common`
  fusion builder. **This reorders `weighted`-fusion results** for existing
  callers that relied on the old implicit 0.5/0.3/0.2 split and did not pass
  explicit weights. `multi_query_search` now also accepts an optional 6th
  argument, `weights: number[] | undefined` (`[avg_weight, max_weight,
  hit_weight]`, must sum to 1.0), to opt back into the old split or any other
  custom weighting — omitting it (as all existing callers do) keeps working
  and simply picks up the new default. (Issue #1545.)
- `relative_score` / `rsf` fusion's per-branch min-max normalization now
  delegates to `velesdb_core::fusion::min_max_normalize` instead of a
  WASM-local reimplementation of the same math. Purely internal
  single-sourcing; output is unchanged. (Issue #1545.)

### Fixed
- **JOIN `ON` condition side order (#1555)**: `ON joined.col = base.col`
  silently matched nothing (every row came back with NULL joined columns)
  because the join key resolution read the two sides of the condition
  positionally. `equality_keys` now orients the condition by which side
  names the joined table (alias or raw table name), mirroring
  `velesdb-core`'s `normalize_join_condition` — both `ON` orders resolve
  identically. Conformance case J005 locks the parity on both engines.
- **SQ8 quantization parity with `velesdb-core` (#1543)**: the WASM SQ8
  encode/decode paths (`store_insert::encode_sq8`, `store_get::decode_sq8`,
  `vector_ops::ScratchBuffer::decode_sq8`) used an ad hoc `1e-10`
  degenerate-range epsilon instead of core's `f32::EPSILON`, and filled
  degenerate (constant/near-constant) vectors with byte `0` per dimension
  instead of core's byte `128`. A near-constant vector could quantize
  differently between the browser and native/server depending on which
  epsilon a given build used. Also fixed: a non-finite range (e.g. a vector
  containing `+Infinity`, or finite min/max whose difference overflows to
  `+Infinity`) could be silently misdecoded as the degenerate case (`min`
  for every dimension) instead of `NaN` for every dimension, which is what
  `velesdb-core` actually produces for that input class.

  **Downgrade hazard**: a `VectorStore` created under `SQ8` mode and
  persisted to `IndexedDB` (`save()`/`export_to_bytes()`) with **this**
  fixed build encodes constant/near-constant vectors as byte `128`
  per-dimension (matching core). Loading that same store back
  (`load()`/`import_from_bytes()`) with an **older** WASM build — i.e.
  downgrading — decodes those bytes using the old build's SQ8 formula,
  which does not special-case byte `128` and will reconstruct the wrong
  values for those specific rows (everything else in the store is
  unaffected). This does not require and does not get a persistence
  format-version bump: the on-disk v2 layout (field count/order/size) is
  unchanged, only which raw bytes an already-degenerate vector encodes to.
  Forward compatibility (older store loaded by this or a newer build) is
  unaffected. Affected users: only those who persisted a `SQ8`-mode store
  containing a constant or near-constant vector with a pre-fix WASM build
  and then downgrade the WASM build while keeping the same `IndexedDB`
  database — re-inserting the affected vectors, or not downgrading,
  avoids the issue entirely.

### Note
- Versions 1.12 through the current 3.12.0 (workspace-wide version bumps)
  are tracked in the workspace root `CHANGELOG.md`, not here; this file's
  per-crate entries resume from this point rather than inventing history
  for the skipped range.

### Added
- **`MemoryService`**: `compileTranscript`, `explainCompilation`,
  `contextSavings`, and `suggestBudget` — the context-compiler tools that
  were previously Node/MCP-only are now reachable in the browser.
  `compileTranscript` deterministically segments a raw agent-session
  transcript into turns (plain marker-based or JSONL) and code/log/body
  sub-segments, then compiles the result exactly like `compileContext`;
  it accepts only an inline `transcript` (no `path` — there is no
  filesystem in WASM). `feedback` and `rememberExtracted` remain
  intentionally absent — see `memory_service.rs`'s module doc for why.
  (#1547)

### Removed
- **Retracted claim**: WASM SIMD128 distance kernels. The `simd.rs` module
  depending on the `wide` crate was not wired into the distance paths used
  by `VectorStore`; distance calculations run on the scalar code paths of
  `velesdb-core` under `wasm32-unknown-unknown`. The `wide` dependency has
  been removed. `wasm-opt --enable-simd` remains enabled as an optimizer
  hint but is not part of the public API contract.

## [1.11.1] - 2026-04-04

### Changed
- Module refactoring: `VectorStore` extracted to `vector_store.rs`, persistence to `vector_store_persistence.rs`
- VelesQL helpers extracted to `velesql_helpers.rs`
- All production files under 500 NLOC (Codacy compliance)
- Version bump to align with workspace v1.11.1 release

## [1.7.0] - 2026-03-24

### Added

#### Performance Optimizations
- **`with_capacity()`** - Create store with pre-allocated memory for known vector counts
- **`reserve()`** - Pre-allocate memory before bulk insertions
- **`insert_batch()`** - Insert multiple vectors in a single call (5-10x faster than individual inserts)

#### Documentation
- High-performance bulk insert examples in README
- Updated API documentation with all new methods
- Interactive browser demo with batch insert

### Changed
- IDs now use `bigint` (u64) instead of `number` for JavaScript interoperability
- Version bump to align with workspace v1.7.0 release

## [0.2.0] - 2025-12-22

### Added

#### Core Features
- **VectorStore** - In-memory vector storage with SIMD-optimized distance calculations
- **Multiple metrics** - Cosine, Euclidean, Dot Product support
- **WASM SIMD128** - Hardware-accelerated vector operations

#### API
- `new(dimension, metric)` - Create vector store
- `insert(id, vector)` - Insert single vector
- `search(query, k)` - Find k nearest neighbors
- `remove(id)` - Remove vector by ID
- `clear()` - Clear all vectors
- `memory_usage()` - Get memory consumption estimate

#### Properties
- `len` - Number of vectors
- `is_empty` - Check if store is empty
- `dimension` - Vector dimension

### Performance
- Insert: ~1µs per vector (128D)
- Search: ~50µs for 10k vectors (128D)
- Memory: 4 bytes per dimension + 8 bytes per ID
