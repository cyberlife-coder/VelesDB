---
phase: 05-sparse-integration
plan: 04
subsystem: api
tags: [rest-api, wasm, sparse-vector, hybrid-search, rrf, serde, utoipa]

requires:
  - phase: 05-01
    provides: "SparseVector type, SparseInvertedIndex, Point.sparse_vectors field"
  - phase: 05-03
    provides: "Hybrid dense+sparse execution with RRF/RSF fusion, execute_sparse_search, execute_hybrid_search"
provides:
  - "REST API sparse vector upsert (dual-format: parallel arrays + dict)"
  - "REST search handler with auto-detect dense/sparse/hybrid mode"
  - "WASM SparseIndex with insert, search, hybrid_search_fuse"
  - "VectorCollection.sparse_search() and hybrid_sparse_search() public methods"
  - "sparse_index top-level module (always compiled, no persistence gate)"
affects: [06-cache, 07-streaming, 08-sdk]

tech-stack:
  added: []
  patterns:
    - "SparseVectorInput with #[serde(untagged)] for dual JSON format acceptance"
    - "Search mode auto-detection from request fields (dense/sparse/hybrid)"
    - "Self-contained WASM sparse index (no velesdb-core persistence dependency)"
    - "Top-level sparse_index module with index::sparse re-export for backward compat"

key-files:
  created:
    - "crates/velesdb-core/src/sparse_index/ (3 files: mod.rs, types.rs, inverted_index.rs, search.rs)"
    - "crates/velesdb-wasm/src/sparse.rs"
  modified:
    - "crates/velesdb-server/src/types.rs"
    - "crates/velesdb-server/src/handlers/points.rs"
    - "crates/velesdb-server/src/handlers/search.rs"
    - "crates/velesdb-core/src/collection/vector_collection.rs"
    - "crates/velesdb-core/src/collection/search/query/hybrid_sparse.rs"
    - "crates/velesdb-core/src/index/sparse/mod.rs"
    - "crates/velesdb-core/src/lib.rs"
    - "crates/velesdb-core/src/point.rs"
    - "crates/velesdb-wasm/src/lib.rs"

key-decisions:
  - "Self-contained WASM sparse index rather than depending on velesdb-core::index::sparse (persistence gate blocks WASM)"
  - "Extracted sparse_index to top-level always-compiled module, index::sparse re-exports from it"
  - "SparseVectorInput uses #[serde(untagged)] enum for transparent dual-format JSON"
  - "Search handler auto-detects mode from presence of vector/sparse_vector fields"
  - "RRF is default fusion strategy for hybrid search via REST (k=60)"

patterns-established:
  - "Dual-format API input: untagged enum accepting both canonical and convenience JSON shapes"
  - "Search mode routing: handler inspects request fields to route to appropriate backend"

requirements-completed: [SPARSE-06, SPARSE-07]

duration: 26min
completed: 2026-03-06
---

# Phase 5 Plan 04: REST API and WASM Sparse Bindings Summary

**REST sparse vector upsert/search endpoints with auto-detect hybrid routing, plus WASM in-memory sparse search bindings**

## Performance

- **Duration:** 26 min
- **Started:** 2026-03-06T22:19:22Z
- **Completed:** 2026-03-06T22:45:26Z
- **Tasks:** 2
- **Files modified:** 14

## Accomplishments

- REST API accepts sparse vectors in upsert (parallel arrays and dict format) with u32 validation at API boundary
- Search handler auto-detects dense/sparse/hybrid mode and routes to appropriate backend
- WASM module exposes SparseIndex with insert, search, and hybrid_search_fuse (RRF fusion)
- VectorCollection gains public sparse_search() and hybrid_sparse_search() methods
- Sparse types always-compiled (extracted to top-level sparse_index module, fixing WASM build)

## Task Commits

Each task was committed atomically:

1. **Task 1: REST API sparse vector types and upsert handler** - `04a67824` (feat)
2. **Task 2: REST search handler sparse/hybrid routing and WASM bindings** - `08880ff9` (feat)

## Files Created/Modified

- `crates/velesdb-server/src/types.rs` - SparseVectorInput, FusionRequest types, extended PointRequest/SearchRequest
- `crates/velesdb-server/src/handlers/points.rs` - Upsert handler converting sparse inputs to Point
- `crates/velesdb-server/src/handlers/search.rs` - Auto-detect dense/sparse/hybrid mode routing
- `crates/velesdb-core/src/collection/vector_collection.rs` - Public sparse_search/hybrid_sparse_search methods
- `crates/velesdb-core/src/collection/search/query/hybrid_sparse.rs` - Promoted helpers to pub(crate)
- `crates/velesdb-core/src/sparse_index/` - Always-compiled sparse module (types, inverted_index, search)
- `crates/velesdb-core/src/index/sparse/mod.rs` - Re-exports from crate::sparse_index
- `crates/velesdb-core/src/lib.rs` - Added sparse_index top-level module
- `crates/velesdb-core/src/point.rs` - Updated import to sparse_index
- `crates/velesdb-core/src/velesql/ast/condition.rs` - Updated import to sparse_index
- `crates/velesdb-core/src/velesql/parser/conditions.rs` - Updated import to sparse_index
- `crates/velesdb-wasm/src/lib.rs` - Registered sparse module
- `crates/velesdb-wasm/src/sparse.rs` - SparseIndex + hybrid_search_fuse WASM bindings

## Decisions Made

- **Self-contained WASM sparse index**: velesdb-core's `index` module is gated behind `persistence` feature, which WASM can't use. Rather than restructuring the entire index module, the WASM sparse bindings implement a self-contained inverted index with DAAT accumulation and RRF fusion. This avoids coupling WASM to persistence-gated code while providing equivalent functionality.
- **Extracted sparse_index to top-level**: Moved sparse types/inverted_index/search to `crate::sparse_index` (always compiled). The `index::sparse` module re-exports from it for backward compatibility. This fixes point.rs and velesql imports that were broken without persistence.
- **Search mode auto-detection**: The handler checks `!req.vector.is_empty()` and `req.sparse_vector.is_some()` to route to dense-only, sparse-only, or hybrid paths. No explicit "mode" field needed.
- **RRF default fusion**: REST hybrid search defaults to RRF with k=60. Clients can override via `fusion: {strategy: "rsf", dense_w: 0.7, sparse_w: 0.3}`.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Extracted sparse module from persistence-gated index to top-level**
- **Found during:** Task 2 (WASM bindings)
- **Issue:** `index::sparse` was behind `#[cfg(feature = "persistence")]`, making sparse types unavailable for WASM and breaking point.rs/velesql imports without persistence
- **Fix:** Created `crate::sparse_index` as always-compiled top-level module; `index::sparse` re-exports from it; updated 3 import sites
- **Files modified:** lib.rs, index/sparse/mod.rs, point.rs, velesql/ast/condition.rs, velesql/parser/conditions.rs, sparse_index/ (new)
- **Verification:** `cargo check --no-default-features` and `cargo check -p velesdb-wasm --no-default-features` both pass
- **Committed in:** 08880ff9

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Essential fix -- sparse types must be available without persistence for WASM and correct compilation. No scope creep.

## Issues Encountered

- **Pre-existing WASM wasm32 build failure**: `cargo build -p velesdb-wasm --no-default-features --target wasm32-unknown-unknown` fails due to `getrandom` crate not supporting wasm32 without special features (rand dependency). This is pre-existing (confirmed by stashing changes). The native `cargo check` verifies code correctness; the wasm32 target issue is a dependency configuration problem outside this plan's scope.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- Phase 5 (Sparse Integration) complete: all 4 plans executed
- Sparse vector support available via REST API, VelesQL, and WASM
- Ready for Phase 6 (Cache) which builds on the write_generation counter

---
*Phase: 05-sparse-integration*
*Completed: 2026-03-06*
