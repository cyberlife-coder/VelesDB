# Core: query plan cache

`velesdb-core` caches compiled query plans in a two-tier LRU cache (L1
lock-free `DashMap` + L2 LRU). Repeated queries skip parsing and planning
entirely when the cache key still matches.

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

---

## How it works

- **Automatic.** The cache is enabled by default on every `Database` instance.
  No configuration required.
- **Write-generation invalidation.** Each collection carries a monotonic write
  generation counter. Inserts, updates and deletes increment it. Cached plans
  whose key embeds a stale generation are bypassed — there is no explicit
  invalidation call to make and no stale-read window to reason about.
- **LRU eviction.** Capacity is bounded; least-recently-used plans are evicted
  when the cache is full.

## Inspecting cache behaviour with `EXPLAIN`

`EXPLAIN` reports `cache_hit` and `plan_reuse_count`:

```sql
EXPLAIN SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10;
```

```json
{
  "query": "SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10",
  "query_type": "SELECT",
  "collection": "docs",
  "plan": [
    { "step": 1, "operation": "VectorSearch", "description": "HNSW search k=10 ef=100", "estimated_rows": 10 }
  ],
  "estimated_cost": {
    "uses_index": true,
    "index_name": "Hnsw",
    "selectivity": 0.001,
    "complexity": "O(log N)"
  },
  "features": {
    "has_vector_search": true,
    "has_filter": false,
    "has_order_by": false,
    "has_group_by": false,
    "has_aggregation": false,
    "has_join": false,
    "has_fusion": false,
    "limit": 10,
    "offset": null
  },
  "cache_hit": true,
  "plan_reuse_count": 42
}
```

- `cache_hit: true` — the plan came from the cache; parsing and planning were
  skipped.
- `cache_hit: false` — cache miss; a fresh plan was compiled and inserted.
- `plan_reuse_count` — how many times this cached plan has been reused across
  all callers.

## Cache metrics

```rust,ignore
let metrics = db.plan_cache().metrics();
println!("Hit rate: {:.1}%", metrics.hit_rate() * 100.0);
println!("Hits: {}, Misses: {}", metrics.hits(), metrics.misses());
```

## Types and methods

| Type | Path | Description |
|------|------|-------------|
| `CompiledPlanCache` | `velesdb_core::cache` | Two-tier cache (L1 lock-free `DashMap` + L2 LRU). Default: 1K L1 / 10K L2 entries |
| `PlanKey` | `velesdb_core::cache` | Cache key: `query_hash: u64`, `schema_version: u64`, `collection_generations: SmallVec<[u64; 4]>` |
| `CompiledPlan` | `velesdb_core::cache` | Cached plan: `plan: QueryPlan`, `referenced_collections: Vec<String>`, `reuse_count: AtomicU64` |
| `PlanCacheMetrics` | `velesdb_core::cache` | `hits()`, `misses()`, `hit_rate() -> f64` (ratio 0.0–1.0) |

| Method | On | Description |
|--------|----|-------------|
| `plan_cache()` | `Database` | Returns `&CompiledPlanCache` |
| `plan_cache().metrics()` | `CompiledPlanCache` | Returns `&PlanCacheMetrics` |
| `plan_cache().stats()` | `CompiledPlanCache` | Returns `LockFreeCacheStats` (L1/L2 sizes, hit counts) |

Full signatures live on [docs.rs](https://docs.rs/velesdb-core).

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Core VelesQL reference](./CORE_VELESQL_REFERENCE.md)
- [Core performance](./CORE_PERFORMANCE.md) — the VelesQL cache-hit micro-benchmark

---

Last updated: 2026-07-25 · Applies to: velesdb-core 6.0.0
