# Core: operations API

The `velesdb-core` surfaces an embedding application reaches for once the
data is in: the control-plane observer and governed reads, query guard-rails,
name and dimension validation, stable ids, index introspection and
maintenance, the filter-strategy decision, the standalone column store, and
the metrics the engine keeps. Each section states what the code enforces
today, including where a setting does less than its name suggests.

Everything here needs the default `persistence` feature unless a section says
otherwise. Full signatures are on [docs.rs/velesdb-core](https://docs.rs/velesdb-core);
[Core API map](./CORE_API_MAP.md) says where each type is imported from.

> The snippets below are function bodies: they use `?`, so paste them inside
> `fn main() -> Result<(), Box<dyn std::error::Error>> { ... Ok(()) }`.

---

## Control-plane observer

`Database::open_with_observer(path, Arc<dyn DatabaseObserver>)` (or
`open_with_observer_and_config`, which also takes a `VelesConfig`) attaches
one observer to the database. Every method of `DatabaseObserver` has a
default — no-op for the telemetry hooks, allow-all for the gates — so an
observer overrides only what it needs, and a database opened without one pays
a single `Option` check per hook.

| Hook | When it fires | Can refuse |
|------|---------------|------------|
| `on_ddl_request(operation, collection)` | before a VelesQL DDL statement: `CREATE`, `DROP`, `CREATE_INDEX`, `DROP_INDEX`, `ANALYZE`, `TRUNCATE`, `ALTER` | yes, by returning `Err` |
| `on_dml_mutation_request(operation, collection)` | before a VelesQL `INSERT_EDGE`, `INSERT_NODE`, `DELETE` or `DELETE_EDGE` | yes, by returning `Err` |
| `on_query_request(&QueryAccessContext)` | before every VelesQL `SELECT` / `MATCH` read, and every `gated_search` / `authorize_read` call | yes, through `AccessDecision` |
| `on_collection_created`, `on_collection_deleted` | after a collection is created or deleted, through VelesQL or the Rust API | no (telemetry) |
| `on_upsert(collection, count)` | after a VelesQL `INSERT` / `UPSERT` | no (telemetry) |
| `on_query(collection, duration_us)` | after a VelesQL read through `Database::execute_query` | no (telemetry) |

The two mutation gates and the `on_upsert` / `on_query` telemetry cover
statements that go through VelesQL (`Database::execute_query`). A direct Rust
call — `Database::create_collection`, `VectorCollection::upsert`, a search on a
collection handle — is never gated, and of the telemetry hooks only creation
and deletion fire for it; a `gated_search` (below) consults
`on_query_request` but does not fire `on_query`.

`on_query_request` returns an `AccessDecision`:

- `Allow` — run the read unmodified (the default);
- `Deny(error)` — abort with that error and no results;
- `AllowWithScope(scope)` — run it with `scope.filter`, a VelesQL
  `Condition`, AND-composed into the query's own filter, so a scope can only
  narrow a result set. `scope.tenant` is forwarded for audit and routing; core
  does not turn it into a filter.

Its `Err` channel is for a failure of the observer itself; a refusal is a
`Deny`. `QueryAccessContext` carries the `collection`, the `operation`
(`VectorSearch`, `TextSearch`, `HybridSearch`, `GraphTraversal`, `Select`) and
two opaque caller hints, `principal` and `tenant_hint`. VelesQL reads reach
the hook with both hints unset; the governed-read entry points below take
them from their caller.

```rust
use std::sync::Arc;
use velesdb_core::velesql::{CompareOp, Comparison, Condition, Value};
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, Error, QueryAccessContext,
};

/// Keeps `audit_*` collections from being dropped, hides `secrets`, and
/// scopes every other read to one tenant's rows.
struct TenantPolicy {
    default_tenant: String,
}

impl DatabaseObserver for TenantPolicy {
    fn on_ddl_request(&self, operation: &str, collection: &str) -> velesdb_core::Result<()> {
        if operation == "DROP" && collection.starts_with("audit_") {
            return Err(Error::Query(format!("{operation} {collection} refused by policy")));
        }
        Ok(())
    }

    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        if ctx.collection == "secrets" {
            return Ok(AccessDecision::Deny(Error::Query("no read access to secrets".into())));
        }
        let tenant = ctx.tenant_hint.unwrap_or(&self.default_tenant);
        let mut scope = AccessScope::default();
        scope.tenant = Some(tenant.to_string());
        scope.filter = Some(Condition::Comparison(Comparison {
            column: "tenant".to_string(),
            operator: CompareOp::Eq,
            value: Value::String(tenant.to_string()),
        }));
        Ok(AccessDecision::AllowWithScope(scope))
    }

    fn on_query(&self, collection: &str, duration_us: u64) {
        eprintln!("read on {collection} took {duration_us} µs");
    }
}

let policy = TenantPolicy { default_tenant: "acme".to_string() };
let db = Database::open_with_observer("./data", Arc::new(policy))?;
```

The types are `#[non_exhaustive]`: match `QueryOperationKind` and
`AccessDecision` with a wildcard arm, and build an `AccessScope` from
`AccessScope::default()` as above. Implementations must be `Send + Sync` and
must not panic. Core ships only the port and the allow-all default; it holds
no policy of its own.

## Governed reads

A `VectorCollection` handle from `Database::get_vector_collection` carries no
observer, so searching it directly bypasses `on_query_request`. Two
`Database` methods put a non-VelesQL read through the same gate.

`gated_search(collection, principal, tenant_hint, GatedRead)` runs a dense,
text or hybrid search after consulting the observer, AND-ing any scope filter
into the caller's own:

```rust
use velesdb_core::{Condition, Filter, GatedRead};

let query = vec![0.1_f32; 768];
let english = Filter::new(Condition::eq("lang", "en"));

let hits = db.gated_search(
    "docs",
    Some("user-42"), // principal hint, passed through untouched
    Some("acme"),    // tenant hint
    GatedRead::Dense { query: &query, k: 10, ef: None, quality: None, filter: Some(&english) },
)?;

let text_hits = db.gated_search(
    "docs", None, None,
    GatedRead::Text { query: "rust embedded database", k: 10, filter: None },
)?;

let hybrid_hits = db.gated_search(
    "docs", None, None,
    GatedRead::Hybrid { vector: &query, text: "rust", k: 10, alpha: Some(0.7), filter: None },
)?;
```

`GatedRead::Dense`'s `ef` and `quality` are honoured only when no filter is in
effect: once the caller or the scope supplies one, the search runs on the
filtered path, which takes neither. With no observer registered,
`gated_search` is the plain search plus one `Option` check.

`authorize_read(collection, operation, principal, tenant_hint)` is the gate
alone, for a read that `GatedRead` does not cover (sparse, batch, multi-query,
graph traversal, `MATCH`): `Ok(None)` means run it unmodified,
`Ok(Some(filter))` means run it with that filter ANDed in, and `Err` means do
not touch the data. A read that cannot apply a metadata filter must refuse on
`Ok(Some(_))` rather than run unfiltered — the server's batch search does
exactly that.

```rust
use velesdb_core::QueryOperationKind;

match db.authorize_read("kg", QueryOperationKind::GraphTraversal, Some("user-42"), None)? {
    None => { /* traverse */ }
    Some(_scope_filter) => { /* traversal cannot filter: refuse */ }
}
```

Which bindings route their reads through this gate is tracked in
[ECOSYSTEM_PARITY.md](../reference/ECOSYSTEM_PARITY.md) (read-path gate parity).

## Guard-rails

Every collection carries guard-rails (`velesdb_core::guardrails`). VelesQL
`SELECT` and `MATCH` execution check the circuit breaker and the per-client
rate limiter before running, and the limits while running; the server's REST
search handlers check the breaker and the rate limiter too.

```rust
use velesdb_core::QueryLimits;

let limits = QueryLimits::new()
    .with_max_depth(5)                      // graph traversal depth
    .with_max_cardinality(10_000)           // intermediate results
    .with_memory_limit(64 * 1024 * 1024)    // bytes per query
    .with_timeout_ms(5_000);                // 0 disables the timeout
db.update_guardrails(&limits);

let docs = db.get_vector_collection("docs").ok_or("collection not found")?;
assert_eq!(docs.guard_rails().limits().max_depth, 5);
```

`QueryLimits::default()` is depth 10, 100 000 intermediate results, 100 MiB,
30 s, a rate limit of 100 000 queries per second per client, and a breaker
opening after 5 consecutive failures for 30 s. What `Database::update_guardrails` reaches:

- `max_depth`, `max_cardinality`, `memory_limit_bytes` and `timeout_ms`
  change for the next query on every collection open at the time of the call;
- `rate_limit_qps`, `circuit_failure_threshold` and
  `circuit_recovery_seconds` do not: each collection's `RateLimiter` and
  `CircuitBreaker` keep the values they were built with;
- a collection created or opened afterwards starts from the defaults, and
  nothing is persisted.

The server's `PUT /guardrails` is this call, with the same reach
([REST reference](../reference/api-reference.md#put-guardrails)).
`RateLimiter` and `CircuitBreaker` are also usable on their own:

```rust
use velesdb_core::guardrails::{CircuitBreaker, RateLimiter};

let breaker = CircuitBreaker::new(3, 10); // open after 3 failures, for 10 s
for _ in 0..3 {
    breaker.record_failure();
}
assert!(breaker.check().is_err()); // CircuitOpen { recovery_in_seconds }
breaker.record_success();           // resets the failure count

let limiter = RateLimiter::new(100); // tokens per second, per client id
limiter.check("client-a")?;
```

## Validation and stable ids

These need no feature: they compile on WASM too.

```rust
use velesdb_core::{
    validate_collection_name, validate_dimension, validate_dimension_match,
    MAX_COLLECTION_NAME_LENGTH, MAX_DIMENSION, MIN_DIMENSION,
};

validate_collection_name("docs-v2")?;                // [A-Za-z0-9_-], 1..=128 chars
assert!(validate_collection_name("../evil").is_err());
assert_eq!(MAX_COLLECTION_NAME_LENGTH, 128);

validate_dimension(768)?;                            // 1..=65_536
assert_eq!((MIN_DIMENSION, MAX_DIMENSION), (1, 65_536));
assert!(validate_dimension_match(768, 384).is_err()); // Error::DimensionMismatch
```

The name rules — no leading hyphen, no `.` or `..`, no Windows device name —
and their error are listed under
[VELES-034](../reference/ERROR_CODES.md#veles-034-invalidcollectionname).
A collection's own `limits.max_dimensions` (default 4096) is enforced at
creation on top of this absolute range ([Configuration](./CONFIGURATION.md)).

Derive a numeric id from a string with the canonical FNV-1a functions, never
with a std hasher, whose seed changes per process:

```rust
use velesdb_core::{hash_edge_id, hash_id, hash_id_bytes};

let point_id = hash_id("doc:2026/10/readme.md");     // same u64 in every process
assert_eq!(point_id, hash_id_bytes("doc:2026/10/readme.md".as_bytes()));

// Edge ids: FNV-1a over source, target (little-endian) and label.
// Directed: swapping the endpoints gives another id.
let edge_id = hash_edge_id(1, 2, "CITES");
assert_ne!(edge_id, hash_edge_id(2, 1, "CITES"));
```

`hash_edge_id` is the id VelesQL `INSERT EDGE` derives when no `id` is given,
and the one agent-memory `relate` uses, so re-inserting the same
`(source, target, label)` overwrites rather than duplicates. Which other
components agree with these functions, and which deliberately do not, is
[KNOWN_LIMITATIONS #12](../reference/KNOWN_LIMITATIONS.md#12-string--u64-point-id-hashing-differs-across-components).

## Index introspection and maintenance

```rust
use velesdb_core::OrderByIndexState;

let docs = db.get_vector_collection("docs").ok_or("collection not found")?;

for info in docs.list_indexes() {
    println!("{} {} ({}): {} distinct values", info.label, info.property, info.index_type, info.cardinality);
}
println!("graph property + range indexes: {} bytes", docs.indexes_memory_usage());

// ORDER BY advisor: scalar `ORDER BY <field> LIMIT k` queries that fell back
// to a full sort, counted per field since the collection was opened.
for advice in docs.order_by_index_advice(10) {
    match advice.state {
        OrderByIndexState::Missing => {
            println!("CREATE INDEX ON docs ({}) — {} fallbacks", advice.field, advice.observed_count);
        }
        OrderByIndexState::BuiltButUncovered => {
            println!("{}: indexed, but some rows lack it or hold a non-scalar", advice.field);
        }
    }
}
```

`list_indexes` returns secondary payload indexes (`label` = `"secondary"`),
graph property indexes and graph range indexes. Its per-entry
`memory_bytes` is `0` for every kind today, `cardinality` is `0` for range
indexes, and secondary indexes report `index_type` `"hash"` although they are
B-trees; `indexes_memory_usage` totals the graph property and range indexes
only. The ORDER BY advisor only observes: it never creates an index, and it
tracks at most 1 024 distinct fields.

### Advanced configuration

`VectorCollection::apply_advanced_config` overrides three fields of a
collection's `config.json` after creation. Each argument is three-state:
`None` leaves the field, `Some(None)` clears it, `Some(Some(v))` sets it.

```rust
use velesdb_core::collection::streaming::DeferredIndexerConfig;

let mut deferred = DeferredIndexerConfig::default(); // disabled, threshold 1024
deferred.enabled = true;
deferred.merge_threshold = 4_096;

docs.apply_advanced_config(
    Some(Some(8)),        // PQ rescore: fetch max(8k, k + 32) candidates
    Some(Some(deferred)), // deferred HNSW indexing
    None,                 // async index builder: unchanged
)?;
```

`pq_rescore_oversampling` (default `Some(4)`; `None` or `Some(0)` disables
the full-precision rescore of a Product Quantization collection) is read by
the next search. `deferred_indexing` and `async_index_builder` are written to
`config.json` but read only when the collection is opened, so they apply from
the next `Database::open`; and the async index builder's `segment_count` is
reserved, parsed but not used. The server applies the same fields from a
`POST /collections` body, with the same timing.

### Auto-reindex detection

An `AutoReindexManager` attached to a vector collection compares its HNSW `M`
with the one its current size calls for. It recommends; it never rebuilds.

```rust
use std::sync::Arc;
use velesdb_core::collection::auto_reindex::{AutoReindexConfig, AutoReindexManager};

let mut config = AutoReindexConfig::default(); // ratio 1.5, from 10 000 vectors
config.min_size_for_reindex = 50_000;
docs.attach_auto_reindex(Arc::new(AutoReindexManager::new(config)));

if let Some(check) = docs.check_auto_reindex_divergence() {
    if check.should_reindex {
        println!("M {} is now {:.1}x below the optimal {}", check.current_m, check.ratio, check.optimal_m);
    }
}
```

The policy is mirrored into `config.json` and restored on the next open;
VelesQL's `ALTER COLLECTION docs SET (auto_reindex = true)` (or `false`)
attaches a manager built from the collection's stored policy, or the default
one, with `enabled` set accordingly. After each `upsert_bulk`, a diverged collection logs
a `tracing` info event; acting on it is the caller's decision.
`detach_auto_reindex` removes the manager and returns it.

## Filter strategy and planner

`velesql::decide_filter_strategy` is the one function both the executor and
`EXPLAIN` use to choose between filtering before and after the HNSW search, so
a plan cannot report a strategy that does not run:

```rust
use velesdb_core::velesql::{
    decide_filter_strategy, set_fallback_selectivity_threshold, FilterDecisionMode,
    FilterStrategy, DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD,
};

// Executor side: the selectivity is measured from the pre-filter bitmap.
assert_eq!(decide_filter_strategy(0.005, FilterDecisionMode::Exact, None), FilterStrategy::PreFilterExact);
assert_eq!(decide_filter_strategy(0.30, FilterDecisionMode::Exact, None), FilterStrategy::PreFilter);
assert_eq!(decide_filter_strategy(0.90, FilterDecisionMode::Exact, None), FilterStrategy::PostFilter);

// Plan side, on a collection never analyzed: a fixed threshold decides.
let plan = FilterDecisionMode::Estimated { has_vector_search: true, ef_search: 128, candidates: 10 };
assert_eq!(decide_filter_strategy(0.05, plan, None), FilterStrategy::PreFilter);

// Process-wide, lock-free; returns the previous value. Range [0.0, 1.0].
let previous = set_fallback_selectivity_threshold(0.3)?;
assert_eq!(previous, DEFAULT_FALLBACK_SELECTIVITY_THRESHOLD); // 0.1
```

In exact mode, a selectivity at or below 1% brute-forces the surviving rows,
above 80% post-filters a full HNSW pass, and the band between runs HNSW on the
bitmap. In estimated mode with `CollectionStats` (after `ANALYZE`), a
calibrated cost comparison decides, and a filter keeping half the rows or
more is always post-filtered to protect recall; without stats, the fallback
threshold decides, as
[KNOWN_LIMITATIONS #3](../reference/KNOWN_LIMITATIONS.md#3-filter-strategy-fallback-threshold-is-runtime-tunable-default-01)
records.

`velesql::QueryPlanner` makes the analogous choice for hybrid `MATCH` +
`NEAR` queries — `GraphFirst` below 1% estimated selectivity, `VectorFirst`
above 50%, `Parallel` between (`QueryPlanner::with_thresholds` moves both
bounds). The engine runs its own planner instance; a planner you construct
answers the question but does not steer a query, which `EXPLAIN` shows.

## Column store

`ColumnStore` is the typed, column-oriented table behind payload filtering and
JOINs, also usable on its own: one typed column per field, interned strings,
an optional `Int` primary key, tombstones and row TTLs. Filters return row
indices (`Vec<usize>`) or `RoaringBitmap`s, deleted rows excluded.

```rust
use velesdb_core::{ColumnStore, ColumnType, ColumnValue};

let mut store = ColumnStore::with_primary_key(
    &[("id", ColumnType::Int), ("category", ColumnType::String), ("price", ColumnType::Int)],
    "id",
)?;

// Strings are interned first; a ColumnValue::String holds the id.
let tech = store.string_table_mut().intern("tech");
let books = store.string_table_mut().intern("books");
let report = store.batch_upsert(&[
    vec![("id", ColumnValue::Int(1)), ("category", ColumnValue::String(tech)), ("price", ColumnValue::Int(120))],
    vec![("id", ColumnValue::Int(2)), ("category", ColumnValue::String(books)), ("price", ColumnValue::Int(15))],
    vec![("id", ColumnValue::Int(3)), ("category", ColumnValue::String(tech)), ("price", ColumnValue::Int(40))],
]);
assert_eq!((report.inserted, report.updated, report.failed.len()), (3, 0, 0));

let tech_rows = store.filter_eq_string("category", "tech");  // rows 0 and 2
let mid_price = store.filter_range_int("price", 10, 100);    // EXCLUSIVE: 10 < price < 100
assert_eq!((tech_rows.len(), mid_price.len()), (2, 2));

let cheap_tech = ColumnStore::bitmap_and(
    &store.filter_eq_string_bitmap("category", "tech"),
    &store.filter_range_int_bitmap("price", 10, 100),
);
assert_eq!(cheap_tech.len(), 1); // row 2

store.set_ttl(2, 3_600)?;           // the row with primary key 2 expires in an hour
let expired = store.expire_rows();  // tombstones every row past its TTL
println!("expired: {:?}", expired.pks);
```

`upsert` on an existing key updates only the columns it names; on a
tombstoned key it revives the row. `batch_upsert` does not stop at a failing
row: each failure is reported in `failed` with its key. The filter family
covers `filter_eq_int` / `filter_gt_int` / `filter_lt_int` /
`filter_range_int`, `filter_eq_string` / `filter_in_string`, the array filters
`filter_contains` / `filter_contains_any` / `filter_contains_all`, and
`filter_geo_distance` / `filter_geo_bbox` (parameters in
`velesdb_core::column_store::{GeoDistanceParams, GeoBboxParams}`, distances in
metres), each with a `*_bitmap` form; `bitmap_and` / `bitmap_or` combine them.
Row TTLs use the system clock.

## Observability

The engine keeps its counters in a few plain types; a server or an embedding
application renders them in the Prometheus text format.

```rust
use std::time::Duration;
use velesdb_core::metrics::global_guardrails_metrics;
use velesdb_core::{compute_latency_percentiles, DurationHistogram, OperationalMetrics};

// Your own counters, in the same families the server exports.
let ops = OperationalMetrics::new();
ops.record_vector_query();             // also counts in velesdb_queries_total
let search_seconds = DurationHistogram::new();
search_seconds.observe(0.004);         // seconds

let mut exposition = ops.export_prometheus();
exposition.push_str(&search_seconds.export_prometheus("myapp_search_seconds", "Search duration"));

// What the engine keeps by itself:
exposition.push_str(&global_guardrails_metrics().export_prometheus()); // process-wide
exposition.push_str(&db.graph_metrics_prometheus());  // one sample per graph collection
exposition.push_str(&db.match_metrics_prometheus());  // process-wide MATCH counters

// Percentiles over your own samples.
let samples: Vec<Duration> = (1..=100).map(|i| Duration::from_micros(i * 10)).collect();
let stats = compute_latency_percentiles(&samples);
println!("p50 {:?}  p95 {:?}  p99 {:?}", stats.p50, stats.p95, stats.p99);
```

- `Database::graph_metrics_prometheus()` renders every graph collection's
  `GraphMetrics` (edge counts, inserts, deletes, BFS/DFS traversals and their
  latency histograms), each sample labelled `collection="<name>"`; it returns
  an empty string when there is no graph collection. One collection's metrics
  are also readable directly, through `GraphCollection::metrics()`.
- `Database::match_metrics_prometheus()` renders the single process-wide
  `MATCH` collector: queries, successes, failures, rows returned, and a latency
  histogram written as `_bucket` lines only.
- `global_guardrails_metrics()` counts rejections the engine records itself:
  `LIKE` patterns, parser depth, invalid storage offsets and corrupt WAL
  entries skipped at replay.
- `OperationalMetrics`, `TraversalMetrics` and `DurationHistogram` are
  counters for the caller to feed; the engine does not update them.

The complete list of families the server writes to `GET /metrics`, with the
ones it declares but does not feed yet, is in the
[REST reference](../reference/api-reference.md#get-metrics).

### Retrieval-quality metrics

For offline evaluation against a ground truth; none of them needs a feature.

```rust
use velesdb_core::{
    average_metrics, hit_rate, mean_average_precision, mrr, ndcg_at_k, precision_at_k,
    recall_at_k,
};

let truth: Vec<u64> = vec![1, 2, 3];
let top_3: Vec<u64> = vec![1, 4, 2];          // pass the top-k slice yourself

let recall = recall_at_k(&truth, &top_3);       // 2/3: found / |truth|
let precision = precision_at_k(&truth, &top_3); // 2/3: found / |results|
let first_hit = mrr(&truth, &top_3);            // 1.0: rank 1 is relevant
let graded = ndcg_at_k(&[3.0, 0.0, 2.0], 3);    // graded relevance per returned rank

// Over several queries.
let queries = vec![(truth.clone(), top_3.clone()), (vec![9], vec![7, 8, 6])];
let any_hit = hit_rate(&queries, 3);            // 0.5: one query of two had a hit
let (avg_recall, avg_precision, avg_mrr) = average_metrics(&[truth.clone()], &[top_3.clone()]);

// MAP takes, per query, the relevance of each returned rank and the number of
// relevant items in the corpus, so a missed item lowers the score.
let map = mean_average_precision(&[(&[true, false, true][..], 3)]);
```

None of these truncates to `k`: they score exactly the slices they are given.

## See also

- [Core API map](./CORE_API_MAP.md) — where each type is imported from
- [Search modes](./SEARCH_MODES.md) — `SearchQuality` and the mode grammar
- [Core / Premium split](../CORE_PREMIUM_SPLIT.md) — why the observer port is policy-free
- [REST reference](../reference/api-reference.md) — `/guardrails`, `/metrics`
- [Known limitations](../reference/KNOWN_LIMITATIONS.md)

---

Last updated: 2026-10-29 · Applies to: velesdb-core 7.1.0
