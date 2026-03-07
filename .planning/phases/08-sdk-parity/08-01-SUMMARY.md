---
phase: 08-sdk-parity
plan: 01
subsystem: sdk
tags: [python, pyo3, sparse-vector, pq-training, streaming, hybrid-search]

# Dependency graph
requires:
  - phase: 04-sparse-vector-engine
    provides: SparseVector type, SparseInvertedIndex, sparse_search
  - phase: 07-streaming-inserts
    provides: stream_insert, BackpressureError, DeltaBuffer
  - phase: 02-pq-core-engine
    provides: ProductQuantizer, TRAIN QUANTIZER VelesQL
provides:
  - Python SDK sparse vector parsing (dict[int,float] and scipy.sparse)
  - Python Collection.search() with dense/sparse/hybrid modes
  - Python Database.train_pq() for PQ codebook training
  - Python Collection.stream_insert() for streaming ingestion
  - Python Collection.upsert() sparse_vector field support
affects: [08-sdk-parity, 09-docs]

# Tech tracking
tech-stack:
  added: []
  patterns: [unified-search-signature, database-level-train]

key-files:
  created:
    - crates/velesdb-core/src/collection/search/sparse.rs
  modified:
    - crates/velesdb-python/src/collection.rs
    - crates/velesdb-python/src/collection_helpers.rs
    - crates/velesdb-python/src/lib.rs
    - crates/velesdb-core/src/collection/search/mod.rs

key-decisions:
  - "train_pq placed on Database (not Collection) because TRAIN QUANTIZER requires Database-level execute_query"
  - "search() uses Option<PyObject> for vector param to support backward-compat positional arg"
  - "Public sparse_search_default/hybrid_sparse_search methods added to legacy Collection for SDK access"

patterns-established:
  - "Unified search signature: search(vector=None, *, sparse_vector=None, top_k=10)"
  - "Sparse vector parsing: dict[int,float] and scipy.sparse duck-typing via .toarray()"

requirements-completed: [SDK-01]

# Metrics
duration: 9min
completed: 2026-03-07
---

# Phase 08 Plan 01: Python SDK Sparse/PQ/Streaming Summary

**Python SDK wired to v1.5 core features: unified search with dense/sparse/hybrid modes, PQ training via VelesQL, and streaming insert**

## Performance

- **Duration:** 9 min
- **Started:** 2026-03-07T15:48:25Z
- **Completed:** 2026-03-07T15:57:33Z
- **Tasks:** 2
- **Files modified:** 6

## Accomplishments
- Python Collection.search() now supports dense-only, sparse-only, and hybrid (RRF k=60) modes via optional parameters
- Python Collection.upsert() accepts sparse_vector field (dict[int,float] or dict[str, dict[int,float]])
- Python Database.train_pq() delegates to VelesQL TRAIN QUANTIZER for PQ codebook training
- Python Collection.stream_insert() sends points via streaming channel with backpressure error handling
- All existing Python API calls (search(vec, 10), upsert([...])) continue to work unchanged

## Task Commits

Each task was committed atomically:

1. **Task 1: Add sparse vector parsing helpers and update upsert** - `ef30bcb4` (feat)
2. **Task 2: Add unified search, train_pq, and stream_insert** - `6a957825` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/collection/search/sparse.rs` - Public sparse_search_default and hybrid_sparse_search on legacy Collection
- `crates/velesdb-core/src/collection/search/mod.rs` - Register sparse module
- `crates/velesdb-python/src/collection_helpers.rs` - parse_sparse_vector, parse_sparse_vectors_from_point helpers
- `crates/velesdb-python/src/collection.rs` - Unified search(), stream_insert() methods
- `crates/velesdb-python/src/lib.rs` - train_pq() on Database, RelativeScore repr fix
- `crates/velesdb-mobile/src/lib.rs` - Formatting fix (pre-existing)

## Decisions Made
- train_pq placed on Database (not Collection) because TRAIN QUANTIZER requires Database-level execute_query dispatch; the legacy Collection lacks a Database back-reference
- search() signature uses `vector=None, *, sparse_vector=None, top_k=10` for full backward compatibility (positional `search(vec, 10)` still works)
- Public sparse_search_default and hybrid_sparse_search methods added to legacy Collection (new search/sparse.rs) since internal methods were pub(crate) and inaccessible to the Python crate

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed pre-existing missing RelativeScore match arm**
- **Found during:** Task 1 (compilation check)
- **Issue:** FusionStrategy __repr__ in lib.rs missing match arm for RelativeScore variant (added in Phase 05)
- **Fix:** Added RelativeScore repr formatting
- **Files modified:** crates/velesdb-python/src/lib.rs
- **Committed in:** ef30bcb4 (Task 1 commit)

**2. [Rule 3 - Blocking] Fixed pre-existing formatting in velesdb-mobile**
- **Found during:** Task 1 (pre-commit hook)
- **Issue:** cargo fmt check failed on velesdb-mobile/src/lib.rs (pre-existing formatting drift)
- **Fix:** Ran cargo fmt --all
- **Files modified:** crates/velesdb-mobile/src/lib.rs
- **Committed in:** ef30bcb4 (Task 1 commit)

**3. [Rule 1 - Bug] Added public sparse search methods to legacy Collection**
- **Found during:** Task 2 (wiring search)
- **Issue:** Legacy Collection only had pub(crate) sparse search methods, inaccessible from Python crate
- **Fix:** Created search/sparse.rs with public sparse_search_default and hybrid_sparse_search methods
- **Files modified:** crates/velesdb-core/src/collection/search/sparse.rs, mod.rs
- **Committed in:** 6a957825 (Task 2 commit)

---

**Total deviations:** 3 auto-fixed (1 bug, 2 blocking)
**Impact on plan:** All auto-fixes necessary for correctness and compilation. No scope creep.

## Issues Encountered
None.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Python SDK now has full v1.5 feature coverage (sparse, PQ, streaming)
- Ready for remaining SDK parity plans (08-02 TypeScript, 08-03 Mobile already complete)
- Ready for documentation phase (09-docs)

## Self-Check: PASSED

All 5 key files verified present. Both task commits (ef30bcb4, 6a957825) verified in git log.

---
*Phase: 08-sdk-parity*
*Completed: 2026-03-07*
