# Migrating to VelesDB 8.0.0

VelesDB 8.0.0 is versioned **MAJOR** for four reasons:

- API deprecated one major or more ago is removed, in Rust and in Python.
- Inputs that used to be stored unreadably, dropped or answered wrongly are now
  refused.
- Several bindings now return core's results and core's error codes instead of
  their own.
- Some results change: integration scores are oriented higher-is-closer, and a
  few binding types change shape.

No change below rewrites data you already stored. The one storage change that
moves a file, an unreadable edge snapshot set aside, logs it (section 1). This
guide covers what a caller has to change. The `[8.0.0]` section of
[`CHANGELOG.md`](../../CHANGELOG.md) holds every change, with the reason for
each.

**Downgrading is not safe for graph collections whose edges carry
properties** (section 1). Read that section before you upgrade a store you
may need to roll back.

---

## 1. Storage

### Edge snapshots (`edge_store.bin`)

Up to 7.1.0, a graph collection with any edge carrying properties could not
reload its snapshot. It reopened without its edges, and the next flush with a
non-empty edge WAL overwrote the snapshot. 8.0.0 writes each edge's properties
as a JSON string, so the snapshot reads back.

- **Snapshots without edge properties** keep the same bytes in both
  directions.
- **An unreadable snapshot is set aside, not overwritten.** The first 8.0.0
  open moves it to `edge_store.bin.unreadable-<unix ms>`, bytes intact, and
  opens the collection with the edges the WAL still holds. Edges that 7.1.0
  already overwrote cannot be recovered.
- **Some snapshots now block the open.** A snapshot that cannot be read for
  another reason (permissions, I/O), or cannot be moved aside, fails that
  collection's open with `VELES-011`. `Database::open` logs it and leaves the
  collection out. 7.1.0 opened such a collection without its edges.
- **Do not downgrade** a collection whose edges have properties. 7.x cannot
  decode the 8.0 snapshot. It opens the collection without its edges, and its
  next flush overwrites them.

### Deeply nested values (`VELES-017`)

Payloads and edge properties nested deeper than 124 levels of arrays and
objects are refused before anything is written. The limit is
`velesdb_core::MAX_VALUE_DEPTH`.

- **Deeper than 127 levels:** before 8.0.0, such a payload could be written
  and then fail on every read.
- **125 to 127 levels:** the payload read back, but the agent-memory snapshot
  holding it failed to restore.
- **Upgrading:** a payload a 7.x store holds at 125 to 127 levels stays
  readable. Writing it back unchanged (a VelesQL `UPDATE`, any
  read-modify-write) is now refused: flatten it first.

### `velesdb-wasm` binary format v3

`export_to_bytes` and `save` write `VELS` v3. A build older than 8.0.0 refuses
a v3 blob with `Unsupported version: 3`.

- **v1 and v2 blobs** still load.
- **A v2 quantized store** (`sq8`, `pq`, `rabitq`, `binary`) loads as the
  reconstruction it was searching over: SQ8-decoded values, or 0/1 sign bits.
  Its original f32 values were never saved.
- **A corrupt v2 quantized store fails at load** with `Invalid data: ...`, for
  example one written by the old `insert_batch`.

## 2. Queries and requests that are now refused

| Input | 7.1.0 | 8.0.0 |
|---|---|---|
| A second `SPARSE_NEAR` (under `AND`, `OR` or `USING FUSION`) | silently dropped | `VELES-010` |
| Hybrid dense + sparse with a dense vector of the wrong dimension | sparse hits only | `VELES-004` |
| Hybrid dense + sparse naming an unknown sparse index | dense hits only, or none | `VELES-009` |
| WASM: `USING FUSION` over a single `NEAR` | accepted | `V012` |
| WASM: `NEAR_FUSED ... USING FUSION 'weighted'` or `'rsf'` | ran as `rrf` | `V012` |
| `rate_limit_qps = 0` or `circuit_failure_threshold = 0` in a guard-rail update | accepted, never applied | `400` / `VELES-009` (REST, Tauri), `ValueError` (Python), `VelesError` (mobile) |
| `enable_streaming` with `buffer_size = 0`, a `buffer_size` past `tokio::sync::Semaphore::MAX_PERMITS`, or `flush_interval_ms = 0` | process aborted | `Error::Config` / `VELES-009` (Rust, Tauri, mobile), `400` (REST), `ValueError` (Python) |
| Python: a payload or edge properties nested past 127 levels of lists and dicts, or containing themselves | segmentation fault | `ValueError` |
| MATCH `ORDER BY` an aggregate (`COUNT(*)`, `MAX(year)`) or a bare name (a pattern alias, a `RETURN` alias, an unqualified property) | `VELES-018` after the traversal ran | `V015` before it: `VELES-010` (Rust, `/collections/{name}/match` `400`), `422 VELESQL_VALIDATION_ERROR` (`/query`), an error (WASM) |

**Fix:**
- Run one query per sparse vector and fuse the results.
- Send a dense vector of the collection's dimension.
- Drop `USING FUSION` from a single-`NEAR` query.
- Use `rrf`, `average` or `maximum` with `NEAR_FUSED`.
- Give streaming a `buffer_size` of at least 1 and a `flush_interval_ms` of at
  least 1.
- Flatten a Python value nested past 127 levels. Core refuses past 124 levels
  anyway, with `VELES-017` (section 1).

The guard-rail limits are also applied now, where 7.1.0 returned them without
applying them: `rate_limit_qps`, `circuit_failure_threshold` and
`circuit_recovery_seconds`. A limit you configured can start rejecting
requests.

**Fix** a MATCH `ORDER BY`: qualify a property with its pattern alias
(`ORDER BY d.year`), or order by `similarity()`, `similarity(field, $v)`,
`depth` or arithmetic over a property (`year - 2000`).

### REST statuses

| Failure | 7.1.0 | 8.0.0 |
|---|---|---|
| A depth, cardinality or capacity ceiling (`max_collections`, `max_dimensions`, `max_vectors_per_collection`, `max_payload_size`, `max_perfect_mode_vectors`), on routes answering with the canonical mapping (`/collections/{name}/match`, collection and point routes) | `503` | `400` |
| The rate limit, on `/collections/{name}/match` | `503` | `429` |
| A storage, I/O or internal fault in a statement on `/query`, `/aggregate` or `/query/explain` | `422` | `500` |
| A timeout under the server's `timeout_ms` on `/query`, `/aggregate` or `/query/explain` | `422` | `503` |

Every body keeps its code (`VELES-027`, or the endpoint's `VELESQL_*`).
Retry only a `5xx`: a `4xx` is refused the same way until the request
changes.

The circuit breaker counts only the engine's failures. A client's own
error — a `4xx` above, its own `timeout_ms` or `WITH (timeout_ms = N)`, a read
the access gate denied — no longer opens it for every caller.

## 3. Rust API

### Removed

| Removed | Use instead |
|---|---|
| `GraphCollection::store_node_payload` | `GraphCollection::upsert_node_payload`, same arguments |
| `OperationalMetrics::shared` | `OperationalMetrics::new_arc` |
| `VelesConfig::effective_ef_search` | `SearchConfig::resolved_quality` (`persistence` feature) |
| `velesql::match_planner::CollectionStats` | `velesql::match_planner::MatchGraphStats`, same fields |
| `check_for_updates`, `spawn_update_check` | `check_for_updates_with`, `spawn_update_check_with` |
| `compute_instance_hash` | none |
| `TraversalMetrics` (crate root and `metrics::`) | `GraphCollection::metrics()`, `Database::graph_metrics_prometheus()` |
| `velesdb-migrate` `PineconeConfig::environment` | drop the field |
| `velesdb-server` `AppState::{traversal_metrics, query_limits}` | the limits live in the `Database` |

- **`resolved_quality`** returns the `SearchQuality` to search at, not a
  `usize`. The removed helper returned `usize::MAX` for `perfect`. Without the
  `persistence` feature, compute the value as
  `search.ef_search.unwrap_or_else(|| search.default_mode.ef_search())`.
- **The update check:**
  - The `_with` calls take an `UpdateCheckContext::new(entry, edition)`; add
    `.with_data_dir(dir)` to keep the data directory the old calls took.
  - They report the calling binary instead of `"library"`.
  - `compute_instance_hash` has no replacement: the check derives its install
    hash itself.
- **`PineconeConfig::environment`:** a YAML config that still sets
  `environment:` keeps loading, with the key ignored.
- **`EdgeStore` no longer implements `Serialize`/`Deserialize`.** Persist a
  store with `to_bytes`/`from_bytes` or `save_to_file`/`load_from_file`.
- **`Database::notify_upsert` and `notify_query` stay**, still deprecated.
  Several server routes have no other source of observer telemetry.

### Changed

- **`Error::GuardRail` is `GuardRail { kind, message }`**, where it was
  `GuardRail(String)`. `kind` is a `GuardRailKind` (`Timeout(TimeoutBudget)`,
  `Depth`, `Cardinality`, `Memory`, `Capacity`, `RateLimit`, `CircuitOpen`),
  and `Error::is_caller_error()` reads it. Match `Error::GuardRail { .. }`
  where you matched `Error::GuardRail(_)`, and read `message` where you read
  the string. `GuardRailViolation::Timeout` gains a `budget` field.
- **`Collection::enable_streaming` and `VectorCollection::enable_streaming`
  return `Result<()>`.** They refuse a config the pipeline cannot run with
  (section 2) with `Error::Config`, and leave the current ingester running.
  Handle the result or `?` it.

### Added

`velesdb_core::MAX_JSON_NESTING` (127) and `MAX_VALUE_DEPTH` (124), also
under `validation`. `GuardRailKind`, `TimeoutBudget`, `Error::is_caller_error`,
`CircuitBreaker::record_error`, `QueryValidator::validate_match_clause` and
`Database::gated_search_outcome`.

## 4. Python and the integrations

### The binding

- **Removed search entry points:**
  - The native `Collection.search(...)` keyword method is removed. The
    `velesdb.Collection.search(vector, top_k, ...)` shorthand stays, and no
    longer warns.
  - `SearchOptions(top_k=...)`, its `.top_k` property and `.with_top_k()` are
    removed: use `k=` and `.with_k()`.
- **`Collection.get_graph_store()` is removed.** Use
  `Database.get_graph_collection()` for a persistent graph, or
  `velesdb.GraphStore()` for an in-memory one.
- **`GraphStore.traverse_dfs` runs core's DFS.**
  - It returns one result per node reached, in depth-first order.
  - An empty `relationship_types` list now follows every edge; it followed none
    before.

### The integrations

- **Version floors:**
  - LangChain, LlamaIndex and Haystack require `velesdb>=5.1.0`.
  - `velesdb_common` needs the 8.0.0 binding for
    `stable_hash_id(algorithm="fnv1a")`, which raises `ImportError` with an
    older wheel.
  - With an older wheel, a search mode is checked only at search time.
- **LangChain `GraphLoader`** writes one persistent graph collection, named
  `collection_name`. A name the previous loader created is not a graph
  collection, so loading into it fails: delete it or pick a new name.
- **Relevance scores are higher-is-closer under every metric.**
  - This applies to LangChain `similarity_search_with_relevance_scores`, its
    `score_threshold` retriever and `GraphRetriever.score_threshold`, and to
    Haystack `Document.score` when `scale_score=True`.
  - A distance `d` (`euclidean`, `hamming`) is returned as `1 / (1 + d)`.
  - A threshold written as a maximum distance `d` becomes `1 / (1 + d)`.
- **LlamaIndex similarity cutoffs need re-tuning:**
  - on `euclidean` and `hamming` stores, for fused, hybrid and text scores;
  - on `jaccard` stores.

  Those scores now pass through as core returns them.

## 5. Tauri plugin

Rejections carry core's codes and a new `recoverable` field:

| 7.1.0 code | 8.0.0 code |
|---|---|
| `NOT_FOUND` | `VELES-003` |
| `DIMENSION_MISMATCH` | `VELES-004` |
| `INVALID_CONFIG` | `VELES-009` |
| `SERIALIZATION_ERROR` | `VELES-012` |
| VelesQL parse error, as `INVALID_CONFIG` | `VELES-010` |
| `train_pq` or aggregation failure, as `INVALID_CONFIG` | core's own code |

- **Messages** are core's (`Database error: [VELES-0xx] ...`). The old prefixes
  `VelesQL parse error:`, `PQ training failed:` and `Aggregation error:` are
  gone.
- **guest-js:** `CommandError` gains `recoverable: boolean`.
- **`TASK_JOIN`** keeps its code.

## 6. Mobile (Swift, Kotlin)

### `MobileGraphStore` is a core graph collection

**API:**
- The constructor, `save(path)` and `MobileGraphStore.load(path)` are gone.
- Create the collection with `createGraphCollection(name)`, or
  `createGraphCollectionWithEmbeddings` for nodes with vectors. Then get its
  store with `VelesDatabase.getGraphStore(name)`. It returns nil/null for a
  missing collection, and throws for one that is not a graph collection.
- The graph is durable without a save step. `flush()` is new.
- Every method now throws.

**Core's rules apply:**
- `addEdge` refuses an edge whose source or target was never added, and an
  empty label.
- `properties_json` must be a JSON object without the reserved `_labels` key.
- `removeEdge(edgeId)` returns a `Bool`: `true` when it removed an edge.
- `addNode` on an existing ID replaces its label and properties. A `vector`
  replaces the embedding, and `nil` keeps it.
- `bfsTraverseParallel` reports a node reached from two sources once per
  source.
- `getOutgoing` and `getIncoming` return edges in core's order.

**Migration:** an app holding a graph saved by the old `save(path)` calls
`importSnapshot(path)` once on the new store.

### Errors

- **The `Collection` and `DimensionMismatch` variants gain `code` and
  `recoverable`.** A Swift pattern or Kotlin destructuring that binds every
  field must bind the two new ones.
- **Some calls raise those two variants instead of `Database`:**
  `executeQuery`, `trainPq`, sparse search and multi-query search. A `catch`
  on `Database` alone misses them.
- **Messages lead with the code.**
- **A lookup of a collection of the wrong kind** answers `VELES-002`.
- **`updateGuardrails` throws `VelesError`** for limits core refuses.
- **`SearchQuality.Adaptive` with `min_ef > max_ef`** is refused.

## 7. `velesdb-wasm` storage modes

- **Every mode keeps f32 vectors** and returns core's top-k and scores for that
  mode.
- **Memory:** 3080 bytes per 768-D vector in every mode. Before, `sq8`, `pq`
  and `rabitq` took about 784 bytes, and `binary` about 104. No mode saves
  memory in the browser any more.
- **Trained PQ collections differ from core:** once a PQ collection has
  trained, core's default `search()` returns ADC-approximated scores. The
  browser returns exact ones.

## 8. Observability

- **`/metrics` no longer writes three families:**
  `velesdb_traversal_nodes_visited_total`,
  `velesdb_traversal_edges_scanned_total` and `velesdb_traversal_max_depth`.
- **Use the per-collection `velesdb_graph_traversals_total` and
  `velesdb_graph_traversal_nodes_visited_total` instead.** Update dashboards and
  alerts that read the removed families.
- **A secondary index's `index_type` is `"btree"`**, where it was `"hash"`.
  This holds in `list_indexes`, REST, the CLI, Python, mobile and Tauri. Match
  `label == "secondary"` or `"btree"`.

## 9. Results that change (not breaking)

- **`MATCH` range predicates return every matching node.** The
  `label.property` range index used to miss two kinds of nodes:
  - nodes written through `upsert`, `upsert_metadata`, `upsert_bulk`,
    `upsert_bulk_from_raw` or `upsert_node(.., Some(vector))`;
  - every node written before a reopen.

  The index now holds every scalar property of every labelled node, strings
  included, and is rebuilt from the stored payloads at open. The
  CHANGELOG measures the cost on 100 000 labelled nodes, each carrying a 1 KB
  string, in a debug build with and without the rebuild: 170 MiB against
  46 MiB of RSS, and an open of 23.8 s against 20.9 s.
- **A bulk upsert of a point without a payload keeps its payload indexed.**
  `upsert` keeps its documented behaviour: a point without a payload deletes
  it.
- **`upsert_bulk` maintains the secondary indexes**, as
  `upsert_bulk_from_raw` already did.
- **VelesQL query results:**
  - `vector NEAR $v OR <metadata>` returns the NEAR hits.
  - `NEAR $d OR SPARSE_NEAR $s` runs the dense + sparse fusion.
  - `traverse_bfs_parallel` counts only results against `limit`.
- **CLI:**
  - `weighted` fusion uses core's 0.6 / 0.3 / 0.1 weights, so its results may
    be ordered differently.
  - `query execute --format json` prints `[]` for an empty result.
- **Stored floats read back bit for bit.** `velesdb-core` enables
  `serde_json`'s `float_roundtrip` feature, so a payload or edge-property
  float reads back as written after a reopen. A value an earlier version
  already read one ULP off and wrote back keeps that value.
  - Cargo unifies the feature across a build: every `serde_json` float parse
    in a binary that links core pays it, your own included. The
    CHANGELOG entry "Stored floats read back bit for bit" states the
    measured cost and the benchmark that reproduces it.
  - An `f32` is parsed directly, so a vector component can differ from 7.1.0
    by one `f32` ULP, toward the exact value.
- **Caller-chosen sizes no longer abort the process.** A huge scroll
  `batch_size`, streaming `batch_size` or HNSW `max_elements` reserved that
  much memory up front. They are now capacity hints: an HNSW index reserves
  at most 2 000 000 slots before its first insert and grows past them as
  nodes arrive.
- **WASM `SELECT DISTINCT`** applies `ORDER BY` and the default `LIMIT`.
- **Error messages:** REST and WASM report core's wording for an unknown fusion
  strategy or search mode.
