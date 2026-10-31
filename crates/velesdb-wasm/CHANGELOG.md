# Changelog

All notable changes to `velesdb-wasm` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

### Changed

- **BREAKING (behavior) — an unaliased aggregate is named as in core.**
  `COUNT(*)` comes back as `count` (was `count(*)`), `SUM(price)` as
  `sum_price` (was `sum(price)`), and every other aggregate as
  `<function>_<column>` (`max_score` for `MAX(score)`). Alias an aggregate
  (`COUNT(*) AS n`) to name it the same in every version.
- **BREAKING (behavior) — GROUP BY and aggregates group as core does**,
  through core's `velesql::Grouper`: a selected column outside `GROUP BY` is
  no longer added to each group row; `MIN`/`MAX` read numeric values and
  return `null` for a group of strings, and `SUM` over no numeric value is
  `null` (was `0`); `score` as an aggregate argument
  outside a vector `GROUP BY` reads a payload field named `score`. A vector
  `GROUP BY` (`NEAR` + `GROUP BY`) returns each group with its best row's id
  and the group score (was `0` and `0.0`), best score first without ORDER BY,
  and 10 groups without LIMIT (was every group).
- **BREAKING (behavior) — a `GROUP BY` ORDER BY that names no column of the
  grouped rows fails with `V016`**, where it was ignored (or, for a selected
  column outside `GROUP BY`, sorted by each group's first value). **An
  aggregate query whose output columns share a name fails with `V017`**,
  where one column overwrote the other: a `GROUP BY` or selected column and
  an aggregate's name, two aggregates, or an aggregate named `id` or `score`
  in a vector `GROUP BY` query.
- **BREAKING (behavior) — HAVING fails with `V018` where it cannot be
  evaluated**: without `GROUP BY` (it filtered the single implicit group),
  or with a threshold that is not a number (it matched no group). `HAVING a
  OR b AND c` reads as `a OR (b AND c)`, as in SQL.
- **BREAKING (behavior) — SQL `GROUP BY` results no longer depend on scan
  order**, as in core: without ORDER BY groups come by `GROUP BY` key
  ascending (was first-seen order), groups an ORDER BY ties come by key,
  `FIRST(col)` reads the row with the smallest point id (the most similar
  row, smallest id on a tie, in a vector `GROUP BY`), and `1` and `1.0` are
  one group.
- **BREAKING (behavior) — a SQL `GROUP BY` holds at most `WITH (max_groups =
  N)` groups, 10,000 by default**, as in core; past it the query fails with
  `Too many groups (limit: N)`.
- **`ORDER BY similarity()` follows the metric's direction**: under a
  distance metric `DESC` puts the nearest row first, as in core.
- **ORDER BY compares values as core does** (`velesql::compare_json_values`):
  `null` stays last under `ASC`, and values of different types sort by type
  (booleans, numbers, strings, arrays, objects) where a number and a string,
  or two booleans, compared equal.
- **BREAKING (behavior) — VelesQL queries go through `velesdb-core`'s
  `QueryValidator`.** A query core refuses is refused here too. `USING FUSION`
  over a single `NEAR` now fails with V012 for every strategy (`rrf`, `maximum`
  and `average` were accepted as a "no-op", but `rrf` re-ranked the hits by RRF
  against a constant metadata branch), and `NEAR_FUSED` with `weighted` or
  `rsf` fails with V012 instead of running as `rrf`. Drop the clause from a
  single-`NEAR` query; give `NEAR_FUSED` `rrf`, `average` or `maximum`.
- **`SELECT DISTINCT` runs in the plain SELECT pipeline** through core's
  `apply_distinct`: `ORDER BY`, `OFFSET` and the default `LIMIT` of 10 now
  apply to distinct rows, `DISTINCT *` collapses rows whose payloads are equal
  (it used to key on id and score), and rows keep their point id instead of 0.
- **BREAKING (behavior) — every storage mode returns `velesdb-core`'s results
  and keeps f32 vectors.** `sq8`, `pq` and `rabitq` stores kept a per-vector
  SQ8 encoding and `binary` stores packed sign bits, and every search scored
  that lossy reconstruction (`binary` as vectors of 0/1): different scores and
  orderings than a core collection with the same mode, and `get()` returned the
  reconstruction. Core keeps f32 in every mode and re-ranks with exact f32, so
  the store now keeps the inserted f32 vectors in every mode and scores them
  exactly. `quantization_parity_tests` checks top-k ids and scores against a
  real core collection for each mode and for cosine, euclidean and dot, through
  core's exact `search_with_quality(Perfect)` (1 200 points, so core's lazy
  SQ8/RaBitQ/PQ quantizers have trained) and its default `search()`.
  **Memory cost:** `memory_usage()` is now 8 bytes per id plus 4 per dimension
  in every mode — 3080 bytes per 768-D vector, where `sq8` took ~784 and
  `binary` ~104. The quantized encoders and decoders (`encode_sq8`,
  `encode_binary`, two copies of the SQ8 decode, `hybrid_quantized.rs`) are
  gone; `batch_search` no longer refuses non-`full` stores; the `pq`/`rabitq`
  "falls back to SQ8" `console.warn` is gone, there being no fallback.
- **Binary format v3.** `export_to_bytes`/`save` write `VELS` v3, the v2 layout
  minus the `data_sq8`/`data_binary`/`sq8_mins`/`sq8_scales` blobs (55 bytes for
  one 3-D vector, was 87). v2 and v1 still load: a v2 quantized store is rebuilt
  as the reconstruction it was searching over, decoded through core's
  `QuantizedVector::to_f32` and `BinaryQuantizedVector`, so its scores match
  the old build to float rounding (frozen v2 exports in `serialization_tests`
  pin this per mode). Its original f32 values were never saved; re-insert them
  for exact scores. An older build refuses a v3 blob (`Unsupported version: 3`)
  instead of loading a quantized store with no codes.

- **Search modes and fusion names are read with `velesdb-core`'s grammar.**
  `search_with_quality` validates its mode with core's `parse_search_mode`,
  which no longer needs the `persistence` feature. The local copy in
  `parsing.rs` is gone. The accepted modes are unchanged, but the refusal
  messages are core's (`Unknown search mode '...'. Valid values: ...`). The
  strategy name of `multi_query_search` is read by
  `FusionStrategy::from_name`, so it is the same name grammar as the server,
  CLI, Tauri and Python, and an unknown name reports core's message. The
  N-branch `rsf` aggregation stays local by design (fixture divergence D005).
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

### Removed

- **Retracted claim**: WASM SIMD128 distance kernels. The `simd.rs` module
  depending on the `wide` crate was not wired into the distance paths used
  by `VectorStore`; distance calculations run on the scalar code paths of
  `velesdb-core` under `wasm32-unknown-unknown`. The `wide` dependency has
  been removed. `wasm-opt --enable-simd` remains enabled as an optimizer
  hint but is not part of the public API contract.

### Fixed

- **`similarity()` thresholds without `NEAR` rank and score like core.** The
  passing rows came back in insertion order with score 0.0; each now carries
  its similarity and the rows are ranked best first.
- **A corrupt store is rejected at load instead of aborting at search.**
  `import_from_bytes`/`load` now check that the vector buffer (and, for a v2
  quantized store, the code buffers) hold exactly one row per id. A v2 `sq8`
  store filled through the old `insert_batch` — which appended to the f32
  buffer whatever the mode and left the codes short — loaded, then indexed out
  of bounds on its first search; it now fails with `Invalid data: ...`.
  `insert_batch` itself is correct in every mode now that every mode stores f32.
- **`import_from_bytes` (v1) rejects a size-overflowing blob instead of
  aborting.** `import_v1` computed `count * dimension` and `count * vector_size`
  unchecked. On `wasm32` (32-bit `usize`, `overflow-checks` off in release) a
  hostile or corrupt blob could wrap those products, spoof the length guard, and
  read out of bounds — a panic that `panic = "abort"` turns into a module abort.
  The size arithmetic moved into a checked `v1_layout` helper that returns a
  clean error (matching the v2 path). The capacity hints in `reserve()` and the
  `with_capacity` constructor also use `saturating_mul` so a 32-bit wasm build
  cannot silently under-reserve.
- **`rabitq` storage mode crashed on the first search**: `ScratchBuffer::new`
  allocated its dequantization buffer only for `SQ8`/`Binary`/`ProductQuantization`,
  but `RaBitQ` is a full SQ8 alias in the browser engine (it encodes via
  `encode_sq8` and decodes via `decode_sq8`). A store created with
  `new_with_mode(.., "rabitq")` therefore indexed into an empty buffer on the
  first `search()`/`query()`/`hybrid_search()`, an out-of-bounds panic that
  `panic = "abort"` turns into a module abort. `RaBitQ` now allocates the
  scratch buffer like `SQ8`.
- **Composite metadata filters failed open on a malformed shape**: the
  `and`/`or`/`not` arms of the filter evaluator used `is_none_or`, so a
  condition whose `conditions` key was absent or not an array (e.g. an object
  from a JS typo), or a `not` with no inner `condition`, matched *every* point
  instead of none — the same unfiltered-leak class the unknown-`type` arm was
  hardened against in the previous release. They now fail closed via
  `is_some_and`; a *present* empty array keeps its vacuous result (`and` -> all
  -> true, `or` -> any -> false).
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

- Versions 1.12 through the current workspace version (the crate is
  workspace-versioned; see the root `Cargo.toml`) are tracked in the workspace
  root `CHANGELOG.md`, not here; this file's per-crate entries resume from
  this point rather than inventing history for the skipped range.

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
