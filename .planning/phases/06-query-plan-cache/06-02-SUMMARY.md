---
phase: 06-query-plan-cache
plan: 02
subsystem: database
tags: [cache, plan-cache, query-plan, explain, prometheus, metrics, invalidation]

# Dependency graph
requires:
  - phase: 06-query-plan-cache
    plan: 01
    provides: "PlanKey, CompiledPlan, CompiledPlanCache, write_generation, schema_version"
provides:
  - "Cache lookup/populate in Database::execute_query() hot path"
  - "Database::explain_query() with cache_hit and plan_reuse_count"
  - "Database::build_plan_key() deterministic key construction"
  - "Prometheus /metrics endpoint with plan cache counters"
affects: [06-query-plan-cache]

# Tech tracking
tech-stack:
  added: []
  patterns: [cache-aside-pattern, explain-cache-status]

key-files:
  created: []
  modified:
    - crates/velesdb-core/src/database.rs
    - crates/velesdb-core/src/velesql/explain.rs
    - crates/velesdb-core/src/velesql/explain_tests.rs
    - crates/velesdb-core/src/cache/plan_cache_tests.rs
    - crates/velesdb-server/src/handlers/metrics.rs
    - crates/velesdb-server/src/main.rs
    - Cargo.lock

key-decisions:
  - "FxHash of query Debug repr for query_hash (deterministic for identical ASTs, avoids canonicalization complexity)"
  - "Cache-aside pattern: cache on miss, skip re-planning on hit (execution still runs against live data)"
  - "explain_query reads cache but does not populate it (only execute_query populates)"
  - "Promote schema_version() and plan_cache() from pub(crate) to pub for server handler access"
  - "Move /metrics route before .with_state() so handler receives State<Arc<AppState>>"

patterns-established:
  - "Cache-aside in execute_query: build PlanKey, check cache, execute, populate on miss"
  - "EXPLAIN cache status: cache_hit and plan_reuse_count fields on QueryPlan"

requirements-completed: [CACHE-01, CACHE-02, CACHE-03, CACHE-04]

# Metrics
duration: 17min
completed: 2026-03-07
---

# Phase 06 Plan 02: Query Plan Cache Wiring Summary

**Cache-aside plan caching in execute_query with EXPLAIN cache_hit status, write_generation/schema_version invalidation, and Prometheus /metrics export of hits/misses/size/hit_rate**

## Performance

- **Duration:** 17 min
- **Started:** 2026-03-07T11:03:35Z
- **Completed:** 2026-03-07T11:21:02Z
- **Tasks:** 2
- **Files modified:** 7

## Accomplishments
- Cache lookup/populate wired into Database::execute_query() with PlanKey built from FxHash + schema_version + write_generations
- QueryPlan extended with cache_hit and plan_reuse_count fields, populated by explain_query
- Cache invalidation verified: upsert/delete bump write_generation, drop/recreate bumps schema_version
- Prometheus /metrics endpoint exports velesdb_plan_cache_hits_total, velesdb_plan_cache_misses_total, velesdb_plan_cache_size, velesdb_plan_cache_hit_rate

## Task Commits

Each task was committed atomically:

1. **Task 1: Wire cache lookup/populate into execute_query + EXPLAIN cache status** - `68fd74f1` (feat)
2. **Task 2: Expose plan cache metrics via Prometheus /metrics endpoint** - `ae8838e0` (feat)

## Files Created/Modified
- `crates/velesdb-core/src/database.rs` - build_plan_key(), explain_query(), cache wiring in execute_query(), pub visibility for plan_cache()/schema_version()
- `crates/velesdb-core/src/velesql/explain.rs` - cache_hit and plan_reuse_count optional fields on QueryPlan
- `crates/velesdb-core/src/velesql/explain_tests.rs` - Updated struct literals for new QueryPlan fields
- `crates/velesdb-core/src/cache/plan_cache_tests.rs` - 5 integration tests: cache hit, invalidation on write/delete/drop+recreate, EXPLAIN miss
- `crates/velesdb-server/src/handlers/metrics.rs` - State extraction, plan cache Prometheus metrics, test_plan_cache_metrics_in_prometheus_output
- `crates/velesdb-server/src/main.rs` - Moved /metrics route before .with_state() for state access
- `Cargo.lock` - smallvec dependency lock (from Plan 01)

## Decisions Made
- FxHash of query Debug repr for query_hash: avoids need for canonical text representation while being deterministic for identical ASTs
- Cache-aside pattern: cache stores QueryPlan, not execution results. On hit, planning is skipped; execution still runs against live data for correctness
- explain_query is read-only on cache (does not populate): only execute_query populates the cache
- Promoted schema_version() and plan_cache() to pub (were pub(crate) with allow(dead_code) from Plan 01)
- Moved /metrics route registration before .with_state() so the prometheus_metrics handler can extract State<Arc<AppState>>

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- Plan cache is fully wired: lookup, populate, invalidation, EXPLAIN status, Prometheus metrics
- All CACHE requirements (CACHE-01 through CACHE-04) are satisfied
- Ready for Phase 07 or additional cache optimizations

---
*Phase: 06-query-plan-cache*
*Completed: 2026-03-07*
