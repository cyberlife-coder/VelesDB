# Migrating to VelesDB 8.0.0

VelesDB 8.0.0 is versioned **MAJOR** for three reasons:

- API deprecated one major or more ago is removed, in Rust and in Python.
- Inputs that used to be stored or answered wrongly are now refused.
- Several bindings now return core's results and core's error codes instead of
  their own.

Every break below is a compile error, an explicit error, or a result that now
matches what core returns. None of them silently changes data you already
stored.

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
`velesdb_core::MAX_VALUE_DEPTH`. Before 8.0.0, such a payload could be written
and then fail on every read.

A 7.x payload held at 125 to 127 levels stays readable. Writing it back
unchanged (a VelesQL `UPDATE`, any read-modify-write) is refused: flatten it
first.

### `velesdb-wasm` binary format v3

`export_to_bytes` and `save` write `VELS` v3. v1 and v2 blobs still load. A
build older than 8.0.0 refuses a v3 blob with `Unsupported version: 3`.

## 2. Queries that are now refused

- **A second `SPARSE_NEAR` fails with `VELES-010`.** This applies under `AND`,
  under `OR`, and with `USING FUSION`. Before, the second clause was dropped
  silently. Run one query per sparse vector and fuse the results.
- **The WASM VelesQL executor runs core's `QueryValidator`.**
  - `USING FUSION` over a single `NEAR` now fails with `V012`: drop the clause.
  - `NEAR_FUSED ... USING FUSION 'weighted'` or `'rsf'` also fails with
    `V012`: use `rrf`, `average` or `maximum`.

## 3. Rust API

### Removed

Each removed item has a replacement that does what it did:

| Removed | Use instead |
|---|---|
| `GraphCollection::store_node_payload` | `GraphCollection::upsert_node_payload` |
| `OperationalMetrics::shared` | `OperationalMetrics::new_arc` |
| `VelesConfig::effective_ef_search` | `SearchConfig::resolved_quality` (`persistence` feature) |
| `velesql::match_planner::CollectionStats` | `velesql::match_planner::MatchGraphStats` |
| `check_for_updates`, `spawn_update_check` | `check_for_updates_with`, `spawn_update_check_with` with an `UpdateCheckContext::new(entry, edition)` |
| `compute_instance_hash` | none: the update check derives its install hash itself |
| `TraversalMetrics` (root and `metrics::`) | `GraphCollection::metrics()`, `Database::graph_metrics_prometheus()` |
| `velesdb-migrate` `PineconeConfig::environment` | drop the field (YAML configs that still set it keep loading) |

- **`EdgeStore` no longer implements `Serialize`/`Deserialize`.** Persist a
  store with `to_bytes`/`from_bytes` or `save_to_file`/`load_from_file`.
- **`Database::notify_upsert` and `notify_query` stay**, still deprecated.
  Several server routes have no other source of observer telemetry.

### Added

`velesdb_core::MAX_JSON_NESTING` (127) and `MAX_VALUE_DEPTH` (124), also
under `validation`.

## 4. Python and the integrations

- **Removed from the binding:**
  - The native `Collection.search(...)` keyword method. The
    `velesdb.Collection.search(vector, top_k, ...)` shorthand stays, and no
    longer warns.
  - `SearchOptions(top_k=...)`, `.top_k` and `.with_top_k()`: use `k=` and
    `.with_k(`.
  - `Collection.get_graph_store()`: use `Database.get_graph_collection()` for
    a persistent graph, or `velesdb.GraphStore()` for an in-memory one.
- **LangChain, LlamaIndex and Haystack** require `velesdb>=5.1.0`.
  `velesdb_common` features that call core directly need 8.0.0:
  `stable_hash_id(algorithm="fnv1a")` and core-checked search modes. Older
  wheels fall back where one exists, or raise `ImportError`.
- **LangChain `GraphLoader`** writes one persistent graph collection, named
  `collection_name`. A name the previous loader created is not a graph
  collection, so loading into it fails: delete it or pick a new name.
- **Relevance scores are higher-is-closer under every metric.**
  - This covers LangChain `similarity_search_with_relevance_scores`, its
    `score_threshold` retriever and `GraphRetriever.score_threshold`. It also
    covers Haystack `Document.score` with `scale_score=True`.
  - A distance `d` (`euclidean`, `hamming`) becomes `1 / (1 + d)`.
  - A threshold written as a maximum distance `d` becomes `1 / (1 + d)`.
- **LlamaIndex similarity cutoffs need re-tuning on `euclidean` and `hamming`
  stores**, for fused, hybrid and text scores. Those scores now pass through as
  core returns them.

## 5. Tauri plugin

Rejections carry core's codes and a new `recoverable` field:

| Before | After |
|---|---|
| `NOT_FOUND` | `VELES-003` |
| `DIMENSION_MISMATCH` | `VELES-004` |
| `INVALID_CONFIG` | `VELES-009` |
| `SERIALIZATION_ERROR` | `VELES-012` |
| VelesQL parse error as `INVALID_CONFIG` | `VELES-010` |

The guest-js `CommandError` gains `recoverable: boolean`. `TASK_JOIN` keeps its
code.

## 6. Mobile (Swift, Kotlin)

- **`MobileGraphStore` is a core graph collection.**
  - **API:** the constructor, `save(path)` and `load(path)` are gone. Create
    the collection with `createGraphCollection(name)`, then get its store with
    `VelesDatabase.getGraphStore(name)`. Every method now throws.
  - **One-time migration:** an app holding a graph saved by the old
    `save(path)` calls `importSnapshot(path)` once on the new store.
- **The `Collection` and `DimensionMismatch` errors gain `code` and
  `recoverable`.** A Swift pattern or Kotlin destructuring that binds every
  field must bind the two new ones.
- **`updateGuardrails` throws `VelesError`** for limits core refuses.

## 7. `velesdb-wasm` storage modes

Every mode now keeps f32 vectors and returns core's top-k and scores for that
mode. The cost is memory: 3080 bytes per 768-D vector in every mode, against
about 784 for `sq8` and 104 for `binary` before. No mode saves memory in the
browser any more.

## 8. Behaviour worth knowing (not breaking)

- **`MATCH` range predicates return every matching node.** Up to 7.1.0, the
  `label.property` range index missed nodes written through `upsert`,
  `upsert_bulk`, `upsert_metadata` or `upsert_node(.., Some(vector))`, and
  every node written before a reopen. Queries now return those nodes too.
  - The index holds every scalar property of every labelled node, strings
    included.
  - Opening a collection rebuilds the index from the stored payloads, which
    costs memory and time. On 100 000 labelled nodes with a 1 KB string each,
    in a debug build, RSS went from 46 MiB to 170 MiB and the open from 20.9 s
    to 23.8 s.
- **A bulk upsert of a point without a payload keeps its payload indexed.**
  The point stays in the label, range, BM25 and histogram indexes. `upsert`
  keeps its documented behaviour: a point without a payload deletes it.
- **`upsert_bulk` maintains the secondary indexes**, as
  `upsert_bulk_from_raw` already did.
- **CLI `weighted` fusion uses core's 0.6 / 0.3 / 0.1 weights**, so its
  results may be ordered differently.
- **`PUT /guardrails` applies to every collection**, including ones created or
  opened after the update.
