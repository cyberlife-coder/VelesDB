---
phase: 15-langchain-llamaindex-v15-parity
plan: 01
subsystem: sdk
tags: [langchain, python, sparse-vectors, pq, streaming, hybrid-search]

# Dependency graph
requires:
  - phase: 08-sdk-bindings
    provides: "Python SDK with sparse_search, train_pq, stream_insert methods"
provides:
  - "LangChain VelesDBVectorStore with sparse vector support in add_texts/similarity_search"
  - "train_pq() method on VectorStore (delegates to Database.train_pq)"
  - "stream_insert() method on VectorStore (delegates to collection.stream_insert)"
  - "validate_sparse_vector() security utility"
affects: [15-02, sdk-docs]

# Tech tracking
tech-stack:
  added: []
  patterns: ["sparse_vector kwarg passthrough via **kwargs for backward compat"]

key-files:
  created: []
  modified:
    - integrations/langchain/src/langchain_velesdb/vectorstore.py
    - integrations/langchain/src/langchain_velesdb/security.py
    - integrations/langchain/src/langchain_velesdb/__init__.py
    - integrations/langchain/tests/test_vectorstore.py

key-decisions:
  - "sparse_vector passed via kwargs.get() in similarity_search, not as explicit parameter, to preserve VectorStore ABC signature"
  - "stream_insert returns int count rather than list of IDs (streaming semantics)"

patterns-established:
  - "Sparse vector kwarg passthrough: similarity_search(**kwargs) -> _run_vector_search(sparse_vector=) for hybrid dense+sparse"
  - "Database-level operations (train_pq) accessed via self._get_db(), collection-level (stream_insert) via self._get_collection()"

requirements-completed: [SDK-05]

# Metrics
duration: 2min
completed: 2026-03-08
---

# Phase 15 Plan 01: LangChain v1.5 Feature Parity Summary

**LangChain VelesDBVectorStore extended with sparse vector hybrid search, PQ training, and streaming inserts -- all backward compatible with Optional[None] defaults**

## Performance

- **Duration:** 2 min
- **Started:** 2026-03-08T13:31:27Z
- **Completed:** 2026-03-08T13:33:38Z
- **Tasks:** 2
- **Files modified:** 4

## Accomplishments
- add_texts() now accepts optional sparse_vectors parameter for dense+sparse hybrid upserts
- similarity_search() and similarity_search_with_score() pass sparse_vector kwarg through to _run_vector_search() for hybrid RRF search
- train_pq() method added to VelesDBVectorStore, delegating to Database.train_pq()
- stream_insert() method added with full validation and sparse vector support
- validate_sparse_vector() security function validates type, key types, value types, and size limits
- 9 new test methods covering all v1.5 features
- Package version bumped from 0.8.10 to 1.5.0

## Task Commits

Each task was committed atomically:

1. **Task 1: Add sparse vector validation and v1.5 methods to LangChain VectorStore** - `a5b222ae` (feat)
2. **Task 2: Add v1.5 feature tests to LangChain test suite** - `177c1234` (test)

## Files Created/Modified
- `integrations/langchain/src/langchain_velesdb/vectorstore.py` - Added sparse_vectors param to add_texts, sparse_vector to _run_vector_search, train_pq(), stream_insert()
- `integrations/langchain/src/langchain_velesdb/security.py` - Added validate_sparse_vector() and MAX_SPARSE_VECTOR_SIZE constant
- `integrations/langchain/src/langchain_velesdb/__init__.py` - Version bumped to 1.5.0
- `integrations/langchain/tests/test_vectorstore.py` - Added TestV15Features class with 9 test methods

## Decisions Made
- sparse_vector passed via kwargs.get() in similarity_search rather than as explicit parameter to preserve VectorStore ABC signature compatibility
- stream_insert() returns int count (number of points inserted) rather than list of IDs, matching streaming semantics where IDs may be auto-generated

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness
- LangChain v1.5 parity complete for SDK-05
- Ready for plan 15-02 (LlamaIndex integration) if applicable

## Self-Check: PASSED

All 4 modified files exist on disk. Both task commits (a5b222ae, 177c1234) verified in git log.

---
*Phase: 15-langchain-llamaindex-v15-parity*
*Completed: 2026-03-08*
