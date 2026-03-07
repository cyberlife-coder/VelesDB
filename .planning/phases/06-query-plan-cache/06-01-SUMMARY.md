---
phase: 06-query-plan-cache
plan: 01
subsystem: database
tags: [cache, plan-cache, lockfree, smallvec, invalidation, write-generation]

# Dependency graph
requires:
  - phase: 05-sparse-integration
    provides: "Completed sparse vector integration with REST API and VelesQL"
provides:
  - "PlanKey, CompiledPlan, PlanCacheMetrics, CompiledPlanCache types"
  - "Collection.write_generation monotonic counter (bumped on all mutation paths)"
  - "Database.schema_version monotonic counter (bumped on all DDL paths)"
  - "Database.compiled_plan_cache instance (L1=1K, L2=10K)"
  - "Database.collection_write_generation() accessor"
affects: [06-query-plan-cache]

# Tech tracking
tech-stack:
  added: [smallvec]
  patterns: [atomic-counter-invalidation, lock-free-plan-cache]

key-files:
  created:
    - crates/velesdb-core/src/cache/plan_cache.rs
    - crates/velesdb-core/src/cache/plan_cache_tests.rs
  modified:
    - crates/velesdb-core/src/cache/mod.rs
    - crates/velesdb-core/src/collection/types.rs
    - crates/velesdb-core/src/collection/core/lifecycle.rs
    - crates/velesdb-core/src/collection/core/crud.rs
    - crates/velesdb-core/src/database.rs
    - crates/velesdb-core/Cargo.toml
    - Cargo.toml

key-decisions:
  - "SmallVec<[u64; 4]> for PlanKey.collection_generations (stack-allocated for <= 4 collections)"
  - "Arc<CompiledPlan> as cache value type (avoids Clone on CompiledPlan, AtomicU64 reuse_count stays shared)"
  - "write_generation bumps once per batch, not per-item (per user decision)"
  - "Default cache sizing L1=1K, L2=10K (per user decision)"
  - "allow(dead_code) on schema_version/plan_cache/compiled_plan_cache accessors (wired in Plan 02)"
  - "create_collection_typed Graph branch gets its own schema_version bump (inline code, not delegating)"

patterns-established:
  - "Atomic counter invalidation: monotonic counters on Collection (write_generation) and Database (schema_version) for cache invalidation"
  - "Plan cache key includes schema_version + per-collection write generations for deterministic invalidation"

requirements-completed: [CACHE-01, CACHE-02, CACHE-03]

# Metrics
duration: 23min
completed: 2026-03-07
---

# Phase 06 Plan 01: Plan Cache Foundation Summary

**Plan cache types with SmallVec-based PlanKey, LockFreeLruCache-backed CompiledPlanCache, and atomic write_generation/schema_version invalidation counters on all mutation and DDL paths**

## Performance

- **Duration:** 23 min
- **Started:** 2026-03-07T10:36:49Z
- **Completed:** 2026-03-07T10:59:45Z
- **Tasks:** 2
- **Files modified:** 9

## Accomplishments
- PlanKey, CompiledPlan, PlanCacheMetrics, CompiledPlanCache types with Send+Sync guarantees
- Collection.write_generation atomic counter bumped on all 4 write paths (upsert, upsert_bulk, upsert_metadata, delete)
- Database.schema_version bumped on all 7 DDL paths (create_collection, create_collection_with_options, create_vector_collection_with_options, create_graph_collection, create_metadata_collection, create_collection_typed Graph, delete_collection)
- Database holds compiled_plan_cache ready for Plan 02 query-path integration

## Task Commits

Each task was committed atomically:

1. **Task 1: Define plan cache types and CompiledPlanCache wrapper** - `e0cc92b8` (feat)
2. **Task 2: Add write_generation to Collection, schema_version + cache to Database** - `1d2374a6` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/cache/plan_cache.rs` - PlanKey, CompiledPlan, PlanCacheMetrics, CompiledPlanCache types
- `crates/velesdb-core/src/cache/plan_cache_tests.rs` - 10 unit/integration tests
- `crates/velesdb-core/src/cache/mod.rs` - Module registration and re-exports
- `crates/velesdb-core/src/collection/types.rs` - write_generation field and accessor on Collection
- `crates/velesdb-core/src/collection/core/lifecycle.rs` - write_generation initialization in all 5 constructors
- `crates/velesdb-core/src/collection/core/crud.rs` - write_generation bumps on all 4 mutation paths
- `crates/velesdb-core/src/database.rs` - schema_version, compiled_plan_cache fields, DDL bumps, accessors
- `crates/velesdb-core/Cargo.toml` - smallvec dependency
- `Cargo.toml` - smallvec workspace dependency

## Decisions Made
- SmallVec<[u64; 4]> for collection_generations avoids heap allocation for queries touching <= 4 collections
- Arc<CompiledPlan> as LockFreeLruCache value type so AtomicU64 reuse_count stays shared across cache reads
- write_generation bumps once per mutation batch (not per-item) per user decision
- Default cache sizing L1=1K hot entries, L2=10K LRU entries per user decision
- create_collection_typed's inline Graph branch gets its own schema_version bump since it doesn't delegate to create_graph_collection

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking] Fixed TODO governance violations**
- **Found during:** Task 2 (commit attempt)
- **Issue:** `// TODO(CACHE-02):` annotations rejected by pre-commit hook (requires EPIC-XXX or US-XXX format)
- **Fix:** Removed TODO comments from allow(dead_code) annotations
- **Files modified:** crates/velesdb-core/src/database.rs
- **Verification:** Pre-commit hook passes
- **Committed in:** 1d2374a6 (Task 2 commit)

---

**Total deviations:** 1 auto-fixed (1 blocking)
**Impact on plan:** Cosmetic fix for CI compliance. No scope change.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Plan cache types and invalidation counters are in place
- Plan 02 can wire CompiledPlanCache into the query execution path
- All existing tests pass with zero regressions

---
*Phase: 06-query-plan-cache*
*Completed: 2026-03-07*
