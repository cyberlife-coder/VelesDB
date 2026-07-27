# Tauri plugin — command, event and permission reference

*Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0*

Complete surface of [`tauri-plugin-velesdb`](../../crates/tauri-plugin-velesdb/README.md):
every IPC command, every event, the permission model, and the storage/metric
tables. For runnable snippets see the
[recipes guide](./TAURI_PLUGIN_RECIPES.md).

Every command is invoked as `plugin:velesdb|<command>` and takes a single
`request` argument (except `delete_collection`, which takes `name`). All
request and response fields are **camelCase** on the JavaScript side; the Rust
DTOs use `#[serde(rename_all = "camelCase")]`.

The authoritative list lives in `crates/tauri-plugin-velesdb/build.rs`
(`COMMANDS`), and a test in `src/commands_tests.rs` fails the build if
`build.rs`, the `invoke_handler` registration in `src/lib.rs`, and
`permissions/default.toml` ever drift apart.

---

## Collections and points

| Command | Description |
|---------|-------------|
| `create_collection` | Create a vector collection |
| `create_metadata_collection` | Create a metadata-only collection (no vectors) |
| `create_graph_collection` | Create a graph collection (schema or schemaless) |
| `delete_collection` | Delete a collection and all its data |
| `list_collections` | List all collections with metadata |
| `get_collection` | Get info about a specific collection |
| `is_empty` | Check whether a collection has no points |
| `flush` | Flush pending writes to disk |
| `compact_storage` | Compact on-disk storage |
| `update_guardrails` | Update runtime guardrail limits |
| `get_guardrails` | Read the current guardrail limits |
| `scroll_collection` | Paginate through a collection |
| `upsert` | Insert or update vectors with payloads |
| `upsert_metadata` | Insert or update metadata-only points |
| `get_points` | Retrieve points by IDs |
| `delete_points` | Delete points by IDs |

## Search

| Command | Description |
|---------|-------------|
| `search` | Vector similarity search |
| `search_ids` | Vector search returning IDs + scores only (no payload hydration) |
| `batch_search` | Parallel batch vector search (multiple queries) |
| `text_search` | BM25 full-text search |
| `hybrid_search` | Combined vector + text search with RRF fusion |
| `multi_query_search` | Multi-query fusion search (RRF / weighted / average / maximum / relative score) |
| `query` | Execute a VelesQL query |
| `sparse_search` | Sparse-only search (inverted index) |
| `hybrid_sparse_search` | Hybrid dense + sparse search |
| `sparse_upsert` | Insert points carrying a sparse vector |
| `train_pq` | Train product quantization on a collection |

## Knowledge graph

| Command | Description |
|---------|-------------|
| `add_edge` | Add one directed edge |
| `add_edges_batch` | Add several edges in one call |
| `get_edges` | Query edges by label / source / target |
| `traverse_graph` | BFS or DFS traversal from one node |
| `traverse_graph_parallel` | Multi-source parallel BFS with deduplication |
| `get_node_degree` | In-degree and out-degree of a node |

## Secondary indexes

| Command | Description |
|---------|-------------|
| `create_index` | Create a secondary metadata index for faster filtered search |
| `drop_index` | Drop a secondary metadata index (returns `true` if one was removed) |
| `list_indexes` | List indexes on a collection |

## Agent memory

Semantic, episodic and procedural memory, plus TTL, eviction and snapshot
versioning. See the [Agent Memory guide](./AGENT_MEMORY.md) for the concepts.

| Family | Commands |
|--------|----------|
| Semantic | `semantic_store`, `semantic_store_with_ttl`, `semantic_query`, `semantic_delete`, `semantic_dimension`, `semantic_serialize`, `semantic_deserialize` |
| Episodic | `episodic_record`, `episodic_recent`, `episodic_recall_similar`, `episodic_older_than`, `episodic_delete`, `episodic_serialize`, `episodic_deserialize` |
| Procedural | `procedural_learn`, `procedural_recall`, `procedural_reinforce`, `procedural_list_all`, `procedural_delete`, `procedural_serialize`, `procedural_deserialize` |
| Lifecycle | `memory_set_ttl`, `memory_auto_expire`, `memory_evict_low_confidence`, `memory_snapshot`, `memory_load_latest_snapshot`, `memory_load_snapshot_version`, `memory_list_snapshot_versions` |
| VelesQL parity | `memory_query_semantic`, `memory_query_episodic`, `memory_query_procedural` |

## Persistence-only commands

Registered only when the `persistence` feature is on — it is part of the
crate's `default` feature set, so these are available unless you opted out with
`default-features = false`.

| Command | Description |
|---------|-------------|
| `stream_insert` | Streaming bulk insert |
| `enable_streaming` | Enable the streaming insert path on a collection |

---

## Events

The plugin installs a `DatabaseObserver` when the database opens, so
create/delete events fire wherever the change originates — a direct command or
VelesQL DDL routed through `query`.

| Event | Payload | Emitted by |
|-------|---------|------------|
| `velesdb://collection-created` | `{ collection, operation: "created" }` | any collection creation, via the observer |
| `velesdb://collection-deleted` | `{ collection, operation: "deleted" }` | any collection deletion, via the observer |
| `velesdb://collection-updated` | `{ collection, operation, count }` | `upsert`, `upsert_metadata`, `sparse_upsert`, `delete_points`, `stream_insert`, `enable_streaming` |
| `velesdb://operation-progress` | `{ operationId, progress, total, processed, message? }` | **nothing yet** — see below |
| `velesdb://operation-complete` | `{ operationId, success, error?, durationMs? }` | **nothing yet** — see below |

`operation` on `collection-updated` is the name of the command that produced
the change (`"upsert"`, `"upsert_metadata"`, `"sparse_upsert"`,
`"delete_points"`, `"stream_insert"`, `"enable_streaming"`).

> The progress and complete payload types and their emit helpers exist in
> `src/events.rs`, but no command calls `emit_progress` / `emit_complete`
> today. Listening for those two events is harmless and currently silent — do
> not build a progress bar on them yet.

---

## Permissions

Tauri 2 denies every plugin command unless a capability grants it. Grant the
whole plugin:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "velesdb",
  "description": "Lets the frontend call VelesDB plugin commands",
  "windows": ["main"],
  "permissions": ["velesdb:default"]
}
```

Or grant commands one by one — every command has an `allow-<command>` (and a
matching `deny-<command>`) permission, generated at build time by `build.rs`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "velesdb-readonly",
  "description": "Search only, no writes",
  "windows": ["main"],
  "permissions": [
    "velesdb:allow-create-collection",
    "velesdb:allow-upsert",
    "velesdb:allow-search"
  ]
}
```

The generated permission set is documented in
`crates/tauri-plugin-velesdb/permissions/autogenerated/reference.md`, and the
default bundle is declared in
`crates/tauri-plugin-velesdb/permissions/default.toml`.

---

## Storage modes

Passed as `storageMode` on `create_collection`.

| Mode | Compression | Best for |
|------|-------------|----------|
| `full` | 1x (f32) | Maximum accuracy (default) |
| `sq8` | 4x | Good accuracy / memory balance |
| `binary` | 32x | Edge / IoT, massive scale |
| `pq` | Variable | Product quantization, ultra-compact |
| `rabitq` | Variable | RaBitQ binary quantization with rescoring |

See the [quantization guide](./QUANTIZATION.md) for the trade-offs.

## Distance metrics

Passed as `metric` on `create_collection`.

| Metric | Score semantics | Best for |
|--------|-----------------|----------|
| `cosine` | Higher is more similar (1.0 = identical) | Text embeddings (default) |
| `euclidean` | Lower is more similar | Spatial / geographic data |
| `dot` | Higher is more similar | Pre-normalized vectors, max inner product |
| `hamming` | Bit distance | Binary vectors |
| `jaccard` | Set similarity | Set / token overlap |

## Search quality

`search` and `search_ids` accept an optional `quality` string: `fast`,
`balanced`, `accurate`, `perfect`, `auto`, or `custom:<ef>`. It is honoured
only on the persistence path, and it is **ignored when a `filter` is also
supplied** (known limitation #457). See the
[search modes guide](./SEARCH_MODES.md).

---

## Engine configuration

`Builder::with_config` takes a `velesdb_core::config::VelesConfig` value;
`Builder::with_config_path` loads one from a TOML file and fails fast — a
missing, unparsable or invalid file raises `Error::ConfigLoad`, never a silent
fallback to defaults.

Only the engine sections are read (`[search]`, `[hnsw]`, `[storage]`,
`[limits]`, `[quantization]`, `[wal_batch]`); any other top-level table — for
instance a `[server]` section owned by `velesdb-server` in a shared config file
— is ignored rather than rejected. `VELESDB_*` environment variables still
layer on top of the filtered file. Field-by-field reference: the
[configuration guide](./CONFIGURATION.md).

---

## Indicative latencies

These order-of-magnitude figures were carried over from the plugin README.
**No benchmark in this repository reproduces them for the plugin**; the
measured numbers live in `crates/velesdb-core/benches/` and in the
[tuning guide](./TUNING_GUIDE.md). Treat the table as a rough expectation, not
a guarantee.

| Operation | Indicative latency |
|-----------|--------------------|
| Vector search (10k vectors) | < 1 ms |
| Text search (BM25) | < 5 ms |
| Hybrid search | < 10 ms |
| Insert (batch of 100) | < 10 ms |

---

## Error codes

Commands reject with `{ message, code }`.

| Code | Raised by |
|------|-----------|
| `VELES-001` | Collection already exists (from velesdb-core) |
| `VELES-002` | Collection not found |
| `VELES-011` | I/O error |
| `INVALID_CONFIG` | Wrong collection kind, bad quality mode, VelesQL parse error, config load failure |
| `NOT_FOUND` | Agent-memory entry not found |
| `DIMENSION_MISMATCH` | Embedding dimension differs from the stored dimension |
| `SERIALIZATION_ERROR` | Payload could not be (de)serialized |

Other `VELES-XXX` codes are forwarded verbatim from velesdb-core.

---

[Back to the guides index](./README.md) ·
[Plugin README](../../crates/tauri-plugin-velesdb/README.md) ·
[Recipes](./TAURI_PLUGIN_RECIPES.md)
